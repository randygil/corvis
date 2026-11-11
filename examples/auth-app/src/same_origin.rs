use corvis::prelude::*;

use crate::settings::Settings;

/// Refuses a state-changing request that a browser made on behalf of another site. It is the check a route needs when
/// its cookie is an ambient credential: `SameSite=Lax` keeps the cookie out of cross-site subrequests, but a top-level
/// cross-site form `POST` to a login route is answered 200 and the browser stores the `Set-Cookie` of the response,
/// which logs the victim in as the attacker (login CSRF).
///
/// It is an allow-list. A request with a method other than `GET`, `HEAD` or `OPTIONS` passes only when:
///
/// - `Sec-Fetch-Site` is present, once, and is `same-origin` (the site itself) or `none` (the user typed the address or
///   used a bookmark); or
/// - its single `Origin` is one of the origins added with [`allow_origin`](Self::allow_origin); or, when
///   `Sec-Fetch-Site` is absent (older browsers, non-browser clients),
/// - `Origin` is the host the request was sent to, or the request has no `Origin` either (curl, server to server: they
///   carry no browser cookies on behalf of a third party).
///
/// Everything else is refused with a 403: `cross-site`, and also `same-site` (a sibling subdomain, which can run
/// attacker-controlled pages, is another site for this purpose: allow it with `allow_origin` if it is yours), a value
/// that is not one of the four of the specification, a repeated header, a repeated `Origin`.
///
/// When the policy has a [`scheme`](Self::scheme), the `Origin` of that fallback must also use it: `https://app.example.com` is
/// not the site `http://app.example.com` (the application cannot see the scheme of the connection behind a proxy, so it is
/// configured: `https` outside `--dev`).
///
/// The host comparison uses the `Host` header of the request as the application received it. Behind a proxy that
/// rewrites `Host` (to an internal name, say) the browser's `Origin` no longer matches it: list the public origins with
/// `allow_origin` instead of relying on the comparison.
///
/// ```ignore
/// let policy = OriginPolicy::new().scheme("https").allow_origin("https://app.example.com");
/// ```
#[derive(Debug, Clone, Default)]
pub struct OriginPolicy {
    allowed: Vec<String>,
    scheme: Option<String>,
}

impl OriginPolicy {
    pub fn new() -> Self {
        OriginPolicy::default()
    }

    /// The scheme the site is served with (`https`). An `Origin` with another scheme is not this site.
    pub fn scheme(mut self, scheme: impl Into<String>) -> Self {
        self.scheme = Some(scheme.into());
        self
    }

    /// Accepts requests whose `Origin` is exactly `origin` (`https://app.example.com`), besides the host itself.
    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        self.allowed.push(origin.into());
        self
    }

    /// `true` when the request is a state-changing one that does not come from the site itself or an allowed origin.
    /// `sec_fetch_site` and `origins` hold every value of the header, in the order received.
    pub fn refuses(&self, method: &str, sec_fetch_site: &[&str], origins: &[&str], host: Option<&str>) -> bool {
        if matches!(method, "GET" | "HEAD" | "OPTIONS") {
            return false;
        }
        let allowed_origin = match origins {
            [origin] => self.allowed.iter().any(|allowed| allowed == origin),
            _ => false,
        };
        match sec_fetch_site {
            [] => {}
            [site] => return !(site.eq_ignore_ascii_case("same-origin") || site.eq_ignore_ascii_case("none") || allowed_origin),
            // A repeated header is not what a browser sends.
            _ => return true,
        }
        match origins {
            [] => false,
            [origin] => {
                if allowed_origin {
                    return false;
                }
                // The same-origin host: the authority of the origin is the host the request was sent to.
                let parts = origin.split_once("://");
                let scheme_ok = match (&self.scheme, parts) {
                    (Some(expected), Some((scheme, _))) => scheme.eq_ignore_ascii_case(expected),
                    (Some(_), None) => false,
                    (None, _) => true,
                };
                let authority = parts.map(|(_, rest)| rest);
                !(scheme_ok && matches!((authority, host), (Some(authority), Some(host)) if authority.eq_ignore_ascii_case(host)))
            }
            _ => true,
        }
    }
}

/// The guard of the routes: the policy above with the scheme of the deployment (`http` only in `--dev`).
#[injectable]
pub struct SameOriginGuard {
    settings: Arc<Settings>,
}

#[async_trait]
impl CanActivate for SameOriginGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let req = ctx.request();
        let values = |name: &str| -> Vec<&str> { req.headers().get_all(name).iter().map(|v| v.to_str().unwrap_or("\u{fffd}")).collect() };
        let policy = OriginPolicy::new().scheme(if self.settings.dev { "http" } else { "https" });
        if policy.refuses(req.method().as_str(), &values("sec-fetch-site"), &values("origin"), req.header("host")) {
            return Err(ForbiddenException::new("Cross-site request refused"));
        }
        Ok(true)
    }

    /// It decides nothing about who the caller is: `AuthGuard` is the guard that controls access.
    fn controls_access(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_same_origin_and_none_pass_by_fetch_metadata() {
        let guard = OriginPolicy::new();
        for site in ["same-origin", "none"] {
            assert!(!guard.refuses("POST", &[site], &[], Some("app.test")), "{site}");
        }
        for site in ["cross-site", "same-site", "garbage", " cross-site", "same-origin ", "", "Cross-Site, same-origin"] {
            assert!(guard.refuses("POST", &[site], &[], Some("app.test")), "{site:?}");
        }
        // A repeated header, even of good values.
        assert!(guard.refuses("POST", &["same-origin", "same-origin"], &[], Some("app.test")));
        // The fetch metadata decides when it is there, whatever Origin says.
        assert!(guard.refuses("POST", &["cross-site"], &["http://app.test"], Some("app.test")));
        assert!(guard.refuses("POST", &["same-site"], &["http://app.test"], Some("app.test")));
        assert!(!guard.refuses("POST", &["same-origin"], &["http://evil.test"], Some("app.test")));
    }

    #[test]
    fn a_listed_origin_passes_whatever_the_fetch_metadata_says() {
        let guard = OriginPolicy::new().allow_origin("https://admin.example.com");
        for site in ["same-site", "cross-site", "garbage"] {
            assert!(!guard.refuses("POST", &[site], &["https://admin.example.com"], Some("api.example.com")), "{site}");
            assert!(guard.refuses("POST", &[site], &["https://evil.example.com"], Some("api.example.com")), "{site}");
        }
        assert!(guard.refuses("POST", &["same-site"], &["https://admin.example.com", "https://admin.example.com"], None));
    }

    #[test]
    fn without_fetch_metadata_the_origin_must_be_the_host_or_allowed() {
        let guard = OriginPolicy::new().allow_origin("https://app.example.com");
        assert!(!guard.refuses("POST", &[], &["http://app.test:3000"], Some("app.test:3000")));
        assert!(!guard.refuses("POST", &[], &["https://app.example.com"], Some("api.test")));
        assert!(guard.refuses("POST", &[], &["https://evil.test"], Some("app.test:3000")));
        assert!(guard.refuses("POST", &[], &["null"], Some("app.test")));
        assert!(guard.refuses("POST", &[], &["http://app.test:3000"], None));
        assert!(guard.refuses("POST", &[], &["http://app.test", "http://app.test"], Some("app.test")));
        // Neither header: not a browser acting for another site.
        assert!(!guard.refuses("POST", &[], &[], Some("app.test")));
    }

    #[test]
    fn a_scheme_mismatch_is_another_site() {
        let policy = OriginPolicy::new().scheme("https");
        assert!(!policy.refuses("POST", &[], &["https://app.test"], Some("app.test")));
        assert!(policy.refuses("POST", &[], &["http://app.test"], Some("app.test")));
        assert!(policy.refuses("POST", &[], &["ftp://app.test"], Some("app.test")));
        // Without a configured scheme the comparison is by host only.
        assert!(!OriginPolicy::new().refuses("POST", &[], &["http://app.test"], Some("app.test")));
    }

    #[test]
    fn behind_a_proxy_that_rewrites_the_host_the_origin_must_be_listed() {
        let unlisted = OriginPolicy::new();
        assert!(unlisted.refuses("POST", &[], &["https://app.example.com"], Some("app-internal:3000")));
        let listed = OriginPolicy::new().allow_origin("https://app.example.com");
        assert!(!listed.refuses("POST", &[], &["https://app.example.com"], Some("app-internal:3000")));
    }

    #[test]
    fn safe_methods_are_never_refused() {
        let guard = OriginPolicy::new();
        for method in ["GET", "HEAD", "OPTIONS"] {
            assert!(!guard.refuses(method, &["cross-site"], &["https://evil.test"], Some("app.test")));
        }
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            assert!(guard.refuses(method, &["cross-site"], &[], Some("app.test")));
        }
    }
}
