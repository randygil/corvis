use corvis::prelude::*;

/// Refuses a state-changing request that a browser made on behalf of another site. It is the check a route needs when
/// its cookie is an ambient credential: `SameSite=Lax` keeps the cookie out of cross-site subrequests, but a top-level
/// cross-site form `POST` to a login route is answered 200 and the browser stores the `Set-Cookie` of the response,
/// which logs the victim in as the attacker (login CSRF).
///
/// A request with a method other than `GET`, `HEAD` or `OPTIONS` is refused (403) when:
///
/// - `Sec-Fetch-Site` is `cross-site`; or, when that header is absent (older browsers, non-browser clients),
/// - `Origin` is present and is neither the host the request was sent to nor one of the origins added with
///   [`allow_origin`](Self::allow_origin).
///
/// Requests with neither header (curl, server to server) pass: they do not carry a browser's cookies on behalf of a
/// third party.
///
/// ```ignore
/// #[use_guards(SameOriginGuard::new(), PassportGuard::new("local").log_in())]
/// ```
#[derive(Debug, Clone, Default)]
pub struct SameOriginGuard {
    allowed: Vec<String>,
}

impl SameOriginGuard {
    pub fn new() -> Self {
        SameOriginGuard::default()
    }

    /// Accepts requests whose `Origin` is exactly `origin` (`https://app.example.com`), besides the host itself.
    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        self.allowed.push(origin.into());
        self
    }

    /// `true` when the request is a state-changing one from another site.
    pub fn refuses(&self, method: &str, sec_fetch_site: Option<&str>, origin: Option<&str>, host: Option<&str>) -> bool {
        if matches!(method, "GET" | "HEAD" | "OPTIONS") {
            return false;
        }
        if let Some(site) = sec_fetch_site {
            return site.eq_ignore_ascii_case("cross-site");
        }
        let Some(origin) = origin else { return false };
        if self.allowed.iter().any(|allowed| allowed == origin) {
            return false;
        }
        // The same-origin host: the authority of the origin is the host the request was sent to.
        let authority = origin.split_once("://").map(|(_, rest)| rest);
        !matches!((authority, host), (Some(authority), Some(host)) if authority.eq_ignore_ascii_case(host))
    }
}

#[async_trait]
impl CanActivate for SameOriginGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let req = ctx.request();
        if self.refuses(req.method().as_str(), req.header("sec-fetch-site"), req.header("origin"), req.header("host")) {
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
    fn a_cross_site_fetch_is_refused_and_the_others_pass() {
        let guard = SameOriginGuard::new();
        assert!(guard.refuses("POST", Some("cross-site"), None, Some("app.test")));
        for site in ["same-origin", "same-site", "none"] {
            assert!(!guard.refuses("POST", Some(site), None, Some("app.test")), "{site}");
        }
        // The fetch metadata decides when it is there, whatever Origin says.
        assert!(guard.refuses("POST", Some("cross-site"), Some("http://app.test"), Some("app.test")));
        assert!(!guard.refuses("POST", Some("same-origin"), Some("http://evil.test"), Some("app.test")));
    }

    #[test]
    fn without_fetch_metadata_the_origin_must_be_the_host_or_allowed() {
        let guard = SameOriginGuard::new().allow_origin("https://app.example.com");
        assert!(!guard.refuses("POST", None, Some("http://app.test:3000"), Some("app.test:3000")));
        assert!(!guard.refuses("POST", None, Some("https://app.example.com"), Some("api.test")));
        assert!(guard.refuses("POST", None, Some("https://evil.test"), Some("app.test:3000")));
        assert!(guard.refuses("POST", None, Some("null"), Some("app.test")));
        assert!(guard.refuses("POST", None, Some("http://app.test:3000"), None));
        // Neither header: not a browser acting for another site.
        assert!(!guard.refuses("POST", None, None, Some("app.test")));
    }

    #[test]
    fn safe_methods_are_never_refused() {
        let guard = SameOriginGuard::new();
        for method in ["GET", "HEAD", "OPTIONS"] {
            assert!(!guard.refuses(method, Some("cross-site"), Some("https://evil.test"), Some("app.test")));
        }
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            assert!(guard.refuses(method, Some("cross-site"), None, Some("app.test")));
        }
    }
}
