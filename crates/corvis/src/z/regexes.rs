//! The regular expressions of `zod/v4/core/regexes` (`z.regexes.*`), as [`JsRegex`].

use std::sync::LazyLock;

use super::regex::JsRegex;

macro_rules! re {
    ($(#[$m:meta])* $name:ident = $src:expr) => {
        $(#[$m])*
        pub static $name: LazyLock<JsRegex> = LazyLock::new(|| JsRegex::new($src, ""));
    };
    ($(#[$m:meta])* $name:ident = $src:expr, $flags:expr) => {
        $(#[$m])*
        pub static $name: LazyLock<JsRegex> = LazyLock::new(|| JsRegex::new($src, $flags));
    };
}

re!(CUID = r"^[cC][0-9a-z]{6,}$");
re!(CUID2 = r"^[0-9a-z]+$");
re!(ULID = r"^[0-9A-HJKMNP-TV-Za-hjkmnp-tv-z]{26}$");
re!(XID = r"^[0-9a-vA-V]{20}$");
re!(KSUID = r"^[A-Za-z0-9]{27}$");
re!(NANOID = r"^[a-zA-Z0-9_-]{21}$");
re!(
    /// ISO 8601-1 duration.
    DURATION = r"^P(?:(\d+W)|(?!.*W)(?=\d|T\d)(\d+Y)?(\d+M)?(\d+D)?(T(?=\d)(\d+H)?(\d+M)?(\d+([.,]\d+)?S)?)?)$"
);
re!(
    /// ISO 8601-2 duration extensions.
    EXTENDED_DURATION = r"^[-+]?P(?!$)(?:(?:[-+]?\d+Y)|(?:[-+]?\d+[.,]\d+Y$))?(?:(?:[-+]?\d+M)|(?:[-+]?\d+[.,]\d+M$))?(?:(?:[-+]?\d+W)|(?:[-+]?\d+[.,]\d+W$))?(?:(?:[-+]?\d+D)|(?:[-+]?\d+[.,]\d+D$))?(?:T(?=[\d+-])(?:(?:[-+]?\d+H)|(?:[-+]?\d+[.,]\d+H$))?(?:(?:[-+]?\d+M)|(?:[-+]?\d+[.,]\d+M$))?(?:[-+]?\d+(?:[.,]\d+)?S)?)??$"
);
re!(GUID = r"^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})$");
re!(
    /// Any RFC 9562 UUID (`regexes.uuid()`).
    UUID = r"^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$"
);
re!(UUID4 = &uuid_source(4));
re!(UUID6 = &uuid_source(6));
re!(UUID7 = &uuid_source(7));
re!(
    /// Practical email validation (default of `z.email()`).
    EMAIL = r"^(?!\.)(?!.*\.\.)([A-Za-z0-9_'+\-\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$"
);
re!(
    /// HTML5 `input[type=email]`.
    HTML5_EMAIL = r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$"
);
re!(
    /// emailregex.com (RFC 5322).
    RFC5322_EMAIL = r#"^(([^<>()\[\]\\.,;:\s@"]+(\.[^<>()\[\]\\.,;:\s@"]+)*)|(".+"))@((\[[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}])|(([a-zA-Z\-0-9]+\.)+[a-zA-Z]{2,}))$"#
);
re!(
    /// Loose Unicode email.
    UNICODE_EMAIL = r#"^[^\s@"]{1,64}@[^\s@]{1,255}$"#,
    "u"
);
re!(BROWSER_EMAIL = r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$");
re!(EMOJI = r"^(\p{Extended_Pictographic}|\p{Emoji_Component})+$", "u");
re!(IPV4 = r"^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])$");
re!(IPV6 = r"^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:))$");
re!(CIDRV4 = r"^((25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])\.){3}(25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])\/([0-9]|[1-2][0-9]|3[0-2])$");
re!(CIDRV6 = r"^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|::|([0-9a-fA-F]{1,4})?::([0-9a-fA-F]{1,4}:?){0,6})\/(12[0-8]|1[01][0-9]|[1-9]?[0-9])$");
re!(BASE64 = r"^$|^(?:[0-9a-zA-Z+/]{4})*(?:(?:[0-9a-zA-Z+/]{2}==)|(?:[0-9a-zA-Z+/]{3}=))?$");
re!(BASE64URL = r"^[A-Za-z0-9_-]*$");
re!(HOSTNAME = r"^(?=.{1,253}\.?$)[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[-0-9a-zA-Z]{0,61}[0-9a-zA-Z])?)*\.?$");
re!(DOMAIN = r"^([a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]{2,}$");
re!(HTTP_PROTOCOL = r"^https?$");
re!(IDN_EMAIL = r#"^[^\s@"]{1,64}@[^\s@]{1,255}$"#, "u");
re!(E164 = r"^\+[1-9]\d{6,14}$");
re!(DATE = &format!("^{DATE_SOURCE}$"));
re!(BIGINT = r"^-?\d+n?$");
re!(INTEGER = r"^-?\d+$");
re!(NUMBER = r"^-?\d+(?:\.\d+)?$");
re!(BOOLEAN = r"^(?:true|false)$", "i");
re!(NULL = r"^null$", "i");
re!(UNDEFINED = r"^undefined$", "i");
re!(LOWERCASE = r"^[^A-Z]*$");
re!(UPPERCASE = r"^[^a-z]*$");
re!(HEX = r"^[0-9a-fA-F]*$");
re!(MD5_HEX = r"^[0-9a-fA-F]{32}$");
re!(MD5_BASE64 = &fixed_base64(22, "=="));
re!(MD5_BASE64URL = &fixed_base64url(22));
re!(SHA1_HEX = r"^[0-9a-fA-F]{40}$");
re!(SHA1_BASE64 = &fixed_base64(27, "="));
re!(SHA1_BASE64URL = &fixed_base64url(27));
re!(SHA256_HEX = r"^[0-9a-fA-F]{64}$");
re!(SHA256_BASE64 = &fixed_base64(43, "="));
re!(SHA256_BASE64URL = &fixed_base64url(43));
re!(SHA384_HEX = r"^[0-9a-fA-F]{96}$");
re!(SHA384_BASE64 = &fixed_base64(64, ""));
re!(SHA384_BASE64URL = &fixed_base64url(64));
re!(SHA512_HEX = r"^[0-9a-fA-F]{128}$");
re!(SHA512_BASE64 = &fixed_base64(86, "=="));
re!(SHA512_BASE64URL = &fixed_base64url(86));

const DATE_SOURCE: &str = r"(?:(?:\d\d[2468][048]|\d\d[13579][26]|\d\d0[48]|[02468][048]00|[13579][26]00)-02-29|\d{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12]\d|3[01])|(?:0[469]|11)-(?:0[1-9]|[12]\d|30)|(?:02)-(?:0[1-9]|1\d|2[0-8])))";

fn uuid_source(v: u8) -> String {
    format!(r"^([0-9a-fA-F]{{8}}-[0-9a-fA-F]{{4}}-{v}[0-9a-fA-F]{{3}}-[89abAB][0-9a-fA-F]{{3}}-[0-9a-fA-F]{{12}})$")
}

fn fixed_base64(body: usize, padding: &str) -> String {
    format!("^[A-Za-z0-9+/]{{{body}}}{padding}$")
}

fn fixed_base64url(len: usize) -> String {
    format!("^[A-Za-z0-9_-]{{{len}}}$")
}

/// `regexes.uuid(version)`.
pub fn uuid(version: Option<u8>) -> JsRegex {
    match version {
        None => UUID.clone(),
        Some(v) => JsRegex::new(&uuid_source(v), ""),
    }
}

/// `regexes.emoji()`.
pub fn emoji() -> JsRegex {
    EMOJI.clone()
}

/// `util.escapeRegex`.
pub fn escape_regex(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if ".*+?^${}()|[]\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `regexes.mac(delimiter)`.
pub fn mac(delimiter: Option<&str>) -> JsRegex {
    let d = escape_regex(delimiter.unwrap_or(":"));
    JsRegex::new(&format!("^(?:[0-9A-F]{{2}}{d}){{5}}[0-9A-F]{{2}}$|^(?:[0-9a-f]{{2}}{d}){{5}}[0-9a-f]{{2}}$"), "")
}

/// Precision of ISO times (`undefined` = any, `-1` = minutes only, `n` = `n` fraction digits).
fn time_source(precision: Option<i32>) -> String {
    let hhmm = r"(?:[01]\d|2[0-3]):[0-5]\d";
    match precision {
        Some(-1) => hhmm.to_string(),
        Some(0) => format!(r"{hhmm}:[0-5]\d"),
        Some(p) => format!(r"{hhmm}:[0-5]\d\.\d{{{p}}}"),
        None => format!(r"{hhmm}(?::[0-5]\d(?:\.\d+)?)?"),
    }
}

/// `regexes.time({ precision })`.
pub fn time(precision: Option<i32>) -> JsRegex {
    JsRegex::new(&format!("^{}$", time_source(precision)), "")
}

/// `regexes.datetime({ precision, offset, local })`.
pub fn datetime(precision: Option<i32>, offset: bool, local: bool) -> JsRegex {
    let time = time_source(precision);
    let mut opts = vec!["Z".to_string()];
    if local {
        opts.push(String::new());
    }
    if offset {
        opts.push(r"([+-](?:[01]\d|2[0-3]):[0-5]\d)".to_string());
    }
    let time_regex = format!("{time}(?:{})", opts.join("|"));
    JsRegex::new(&format!("^{DATE_SOURCE}T(?:{time_regex})$"), "")
}

/// `regexes.string(bag)` (the bag always exists, so the `{min,max}` form is used).
pub fn string(min: Option<usize>, max: Option<usize>) -> JsRegex {
    let src = format!(r"[\s\S]{{{},{}}}", min.unwrap_or(0), max.map(|m| m.to_string()).unwrap_or_default());
    JsRegex::new(&format!("^{src}$"), "")
}
