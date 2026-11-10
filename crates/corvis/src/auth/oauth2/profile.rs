//! The normalized profile of the user, as `passport-google-oauth20` and `passport-github2` build it, plus a verified
//! flag per e-mail address.

use serde_json::{Map, Value, json};

/// The longest `id` (subject) accepted, in bytes.
const MAX_ID_BYTES: usize = 256;
/// The most e-mail entries kept from the GitHub list.
const MAX_EMAILS: usize = 256;

/// An e-mail address of a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OAuth2Email {
    pub value: String,
    /// Whether the provider reports the address as verified. An address without the flag is not verified.
    pub verified: bool,
    /// Whether it is the account's primary address: GitHub only (`None` for Google and generic profiles).
    pub primary: Option<bool>,
}

/// The name parts of a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OAuth2Name {
    pub family_name: Option<String>,
    pub given_name: Option<String>,
}

/// The normalized profile the provider's user endpoint returned. Identify a user by `(provider, id)`; link an existing
/// account by e-mail only through [`verified_email`](OAuth2Profile::verified_email): an unverified address is
/// something anyone can claim.
///
/// `Serialize` writes the keys of the original (`displayName`, `profileUrl`, `_json`, ...). `Debug` prints the provider
/// only: the rest is personal data.
#[derive(Clone)]
#[non_exhaustive]
pub struct OAuth2Profile {
    /// `"google"`, `"github"`, or the strategy name for a generic provider.
    pub provider: String,
    /// The subject: Google's `sub`, GitHub's numeric `id` as a string. Never empty.
    pub id: String,
    pub display_name: Option<String>,
    pub username: Option<String>,
    pub name: Option<OAuth2Name>,
    pub emails: Vec<OAuth2Email>,
    pub photos: Vec<String>,
    pub profile_url: Option<String>,
    /// GitHub's `node_id`.
    pub node_id: Option<String>,
    /// The provider's document as received (`_json`).
    pub raw: Value,
}

impl std::fmt::Debug for OAuth2Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2Profile").field("provider", &self.provider).finish_non_exhaustive()
    }
}

impl OAuth2Profile {
    /// The primary verified address: for GitHub the entry that is both `primary` and `verified`; for Google and
    /// generic profiles the address when the provider says `email_verified: true`. `None` otherwise.
    pub fn verified_email(&self) -> Option<&str> {
        self.emails.iter().find(|e| e.verified && e.primary.unwrap_or(true)).map(|e| e.value.as_str())
    }

    /// The JSON form, with the keys of the original's profile object.
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("provider".into(), json!(self.provider));
        map.insert("id".into(), json!(self.id));
        if let Some(v) = &self.node_id {
            map.insert("nodeId".into(), json!(v));
        }
        if let Some(v) = &self.display_name {
            map.insert("displayName".into(), json!(v));
        }
        if let Some(v) = &self.username {
            map.insert("username".into(), json!(v));
        }
        if let Some(v) = &self.profile_url {
            map.insert("profileUrl".into(), json!(v));
        }
        if let Some(name) = &self.name {
            let mut n = Map::new();
            if let Some(v) = &name.family_name {
                n.insert("familyName".into(), json!(v));
            }
            if let Some(v) = &name.given_name {
                n.insert("givenName".into(), json!(v));
            }
            map.insert("name".into(), Value::Object(n));
        }
        if !self.emails.is_empty() {
            let list: Vec<Value> = self
                .emails
                .iter()
                .map(|e| {
                    let mut m = Map::new();
                    m.insert("value".into(), json!(e.value));
                    if let Some(primary) = e.primary {
                        m.insert("primary".into(), json!(primary));
                    }
                    m.insert("verified".into(), json!(e.verified));
                    Value::Object(m)
                })
                .collect();
            map.insert("emails".into(), Value::Array(list));
        }
        if !self.photos.is_empty() {
            map.insert("photos".into(), Value::Array(self.photos.iter().map(|p| json!({ "value": p })).collect()));
        }
        map.insert("_json".into(), self.raw.clone());
        Value::Object(map)
    }
}

impl serde::Serialize for OAuth2Profile {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

/// A string member, `None` when it is absent, not a string or empty.
fn text(map: &Map<String, Value>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn empty_profile(provider: &str, id: String, raw: Value) -> OAuth2Profile {
    OAuth2Profile {
        provider: provider.to_string(),
        id,
        display_name: None,
        username: None,
        name: None,
        emails: Vec::new(),
        photos: Vec::new(),
        profile_url: None,
        node_id: None,
        raw,
    }
}

/// The OpenID userinfo mapping of `passport-google-oauth20` (`profile/openid.js`): `sub` must be a non-empty string.
/// `email_verified` counts only as the JSON boolean `true`.
pub(crate) fn openid(document: &Map<String, Value>, provider: &str) -> Option<OAuth2Profile> {
    let id = text(document, "sub").filter(|s| s.len() <= MAX_ID_BYTES)?;
    let mut profile = empty_profile(provider, id, Value::Object(document.clone()));
    profile.display_name = text(document, "name");
    let (family, given) = (text(document, "family_name"), text(document, "given_name"));
    if family.is_some() || given.is_some() {
        profile.name = Some(OAuth2Name { family_name: family, given_name: given });
    }
    if let Some(email) = text(document, "email") {
        let verified = matches!(document.get("email_verified"), Some(Value::Bool(true)));
        profile.emails.push(OAuth2Email { value: email, verified, primary: None });
    }
    if let Some(picture) = text(document, "picture") {
        profile.photos.push(picture);
    }
    Some(profile)
}

/// The `/user` mapping of `passport-github2` (`profile.js`): `id` is a number or a string of digits, rendered as a
/// string. The public address of the document is an unverified e-mail.
pub(crate) fn github_user(document: &Map<String, Value>) -> Option<OAuth2Profile> {
    let id = match document.get("id")? {
        Value::Number(n) => n.as_u64().map(|v| v.to_string())?,
        Value::String(s) if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) => s.clone(),
        _ => return None,
    };
    let mut profile = empty_profile("github", id, Value::Object(document.clone()));
    profile.node_id = text(document, "node_id");
    profile.display_name = text(document, "name");
    profile.username = text(document, "login");
    profile.profile_url = text(document, "html_url");
    if let Some(email) = text(document, "email") {
        profile.emails.push(OAuth2Email { value: email, verified: false, primary: Some(false) });
    }
    if let Some(avatar) = text(document, "avatar_url") {
        profile.photos.push(avatar);
    }
    Some(profile)
}

/// Replaces the e-mails of a GitHub profile with the `/user/emails` list: every entry as `{ value, primary,
/// verified }`. `None` when the list is not a non-empty array with a usable entry (the original fails the login too).
pub(crate) fn github_emails(profile: &mut OAuth2Profile, list: &Value) -> Option<()> {
    let entries = list.as_array()?;
    let mut emails = Vec::new();
    for entry in entries.iter().take(MAX_EMAILS) {
        let Some(entry) = entry.as_object() else { continue };
        let Some(value) = text(entry, "email") else { continue };
        let flag = |key: &str| matches!(entry.get(key), Some(Value::Bool(true)));
        emails.push(OAuth2Email { value, verified: flag("verified"), primary: Some(flag("primary")) });
    }
    if emails.is_empty() {
        return None;
    }
    profile.emails = emails;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn google_documents() {
        let p = openid(
            &doc(json!({ "sub": "1234", "name": "Ana Perez", "given_name": "Ana", "family_name": "Perez", "email": "ana@x.io", "email_verified": true, "picture": "https://x/p.png" })),
            "google",
        )
        .unwrap();
        assert_eq!(p.id, "1234");
        assert_eq!(p.verified_email(), Some("ana@x.io"));
        let j = p.to_json();
        assert_eq!(j["emails"], json!([{ "value": "ana@x.io", "verified": true }]));
        assert_eq!(j["name"], json!({ "familyName": "Perez", "givenName": "Ana" }));
        assert_eq!(j["photos"], json!([{ "value": "https://x/p.png" }]));
        assert_eq!(j["_json"]["sub"], "1234");
    }

    #[test]
    fn an_unverified_google_address_is_not_a_verified_email() {
        for flag in [json!(false), json!("true"), json!(1), Value::Null] {
            let p = openid(&doc(json!({ "sub": "1", "email": "a@x.io", "email_verified": flag })), "google").unwrap();
            assert!(!p.emails[0].verified);
            assert_eq!(p.verified_email(), None);
        }
        let p = openid(&doc(json!({ "sub": "1", "email": "a@x.io" })), "google").unwrap();
        assert_eq!(p.verified_email(), None);
    }

    #[test]
    fn documents_with_missing_fields() {
        let p = openid(&doc(json!({ "sub": "1" })), "google").unwrap();
        assert!(p.emails.is_empty() && p.photos.is_empty() && p.name.is_none() && p.display_name.is_none());
        assert_eq!(p.to_json().as_object().unwrap().keys().collect::<Vec<_>>(), ["provider", "id", "_json"]);
        // a name part alone makes the name object
        let p = openid(&doc(json!({ "sub": "1", "given_name": "Ana" })), "x").unwrap();
        assert_eq!(p.to_json()["name"], json!({ "givenName": "Ana" }));
        // wrong types are absent, not errors
        let p = openid(&doc(json!({ "sub": "1", "name": 5, "email": ["a"], "picture": {} })), "x").unwrap();
        assert!(p.display_name.is_none() && p.emails.is_empty() && p.photos.is_empty());
    }

    #[test]
    fn the_subject_must_be_a_usable_string() {
        for bad in [
            json!({}),
            json!({ "sub": "" }),
            json!({ "sub": 7 }),
            json!({ "sub": null }),
            json!({ "sub": ["a"] }),
            json!({ "sub": "x".repeat(257) }),
        ] {
            assert!(openid(&doc(bad), "google").is_none());
        }
        assert!(openid(&doc(json!({ "sub": "x".repeat(256) })), "google").is_some());
    }

    #[test]
    fn a_generic_profile_carries_the_strategy_name() {
        assert_eq!(openid(&doc(json!({ "sub": "1" })), "corp").unwrap().provider, "corp");
    }

    #[test]
    fn github_ids_are_numbers_or_digit_strings() {
        for (id, want) in [(json!(583231), "583231"), (json!("583231"), "583231"), (json!(0), "0")] {
            assert_eq!(github_user(&doc(json!({ "id": id }))).unwrap().id, want);
        }
        for bad in [json!(-1), json!(1.5), json!("12a"), json!(""), json!(null), json!([1]), json!(true), json!("1".repeat(21))] {
            assert!(github_user(&doc(json!({ "id": bad }))).is_none(), "{bad}");
        }
        assert!(github_user(&doc(json!({}))).is_none());
    }

    #[test]
    fn github_users() {
        let p = github_user(&doc(json!({ "id": 1, "node_id": "N", "login": "octo", "name": "Octo", "html_url": "https://github.com/octo", "email": "o@x.io", "avatar_url": "https://a/1.png" }))).unwrap();
        assert_eq!((p.provider.as_str(), p.username.as_deref(), p.node_id.as_deref()), ("github", Some("octo"), Some("N")));
        assert_eq!(p.emails, vec![OAuth2Email { value: "o@x.io".into(), verified: false, primary: Some(false) }]);
        assert_eq!(p.verified_email(), None);
        let j = p.to_json();
        assert_eq!(j["profileUrl"], "https://github.com/octo");
        assert_eq!(j["nodeId"], "N");
    }

    #[test]
    fn github_emails_and_the_primary_verified_address() {
        let mut p = github_user(&doc(json!({ "id": 1 }))).unwrap();
        let list = json!([
            { "email": "primary-unverified@x.io", "primary": true, "verified": false },
            { "email": "secondary@x.io", "primary": false, "verified": true, "visibility": null },
            { "email": 7 },
            "junk"
        ]);
        github_emails(&mut p, &list).unwrap();
        assert_eq!(p.emails.len(), 2);
        assert_eq!(p.verified_email(), None, "a verified secondary address is not enough");
        let list =
            json!([{ "email": "a@x.io", "primary": true, "verified": true }, { "email": "b@x.io", "primary": false, "verified": true }]);
        github_emails(&mut p, &list).unwrap();
        assert_eq!(p.verified_email(), Some("a@x.io"));
        assert_eq!(p.to_json()["emails"][0], json!({ "value": "a@x.io", "primary": true, "verified": true }));
        // wrong flag types count as false
        let list = json!([{ "email": "a@x.io", "primary": "true", "verified": 1 }]);
        github_emails(&mut p, &list).unwrap();
        assert_eq!(p.verified_email(), None);
    }

    #[test]
    fn an_empty_or_unusable_email_list_fails() {
        let mut p = github_user(&doc(json!({ "id": 1 }))).unwrap();
        for bad in [json!([]), json!({}), json!("x"), json!([{ "primary": true }]), json!([null])] {
            assert!(github_emails(&mut p, &bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn debug_prints_the_provider_only() {
        let p = openid(&doc(json!({ "sub": "SUBJECT-1", "email": "secret@x.io", "name": "Real Name" })), "google").unwrap();
        let printed = format!("{p:?}");
        assert!(
            printed.contains("google") && !printed.contains("SUBJECT") && !printed.contains("secret@") && !printed.contains("Real"),
            "{printed}"
        );
        assert_eq!(serde_json::to_value(&p).unwrap(), p.to_json());
    }
}
