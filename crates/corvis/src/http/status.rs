//! `HttpStatus`: named HTTP status codes (`HttpStatus::CREATED`, `HttpStatus::NOT_FOUND`, ...).

pub struct HttpStatus;

impl HttpStatus {
    pub const CONTINUE: u16 = 100;
    pub const SWITCHING_PROTOCOLS: u16 = 101;
    pub const PROCESSING: u16 = 102;
    pub const EARLYHINTS: u16 = 103;
    pub const OK: u16 = 200;
    pub const CREATED: u16 = 201;
    pub const ACCEPTED: u16 = 202;
    pub const NON_AUTHORITATIVE_INFORMATION: u16 = 203;
    pub const NO_CONTENT: u16 = 204;
    pub const RESET_CONTENT: u16 = 205;
    pub const PARTIAL_CONTENT: u16 = 206;
    pub const AMBIGUOUS: u16 = 300;
    pub const MOVED_PERMANENTLY: u16 = 301;
    pub const FOUND: u16 = 302;
    pub const SEE_OTHER: u16 = 303;
    pub const NOT_MODIFIED: u16 = 304;
    pub const TEMPORARY_REDIRECT: u16 = 307;
    pub const PERMANENT_REDIRECT: u16 = 308;
    pub const BAD_REQUEST: u16 = 400;
    pub const UNAUTHORIZED: u16 = 401;
    pub const PAYMENT_REQUIRED: u16 = 402;
    pub const FORBIDDEN: u16 = 403;
    pub const NOT_FOUND: u16 = 404;
    pub const METHOD_NOT_ALLOWED: u16 = 405;
    pub const NOT_ACCEPTABLE: u16 = 406;
    pub const PROXY_AUTHENTICATION_REQUIRED: u16 = 407;
    pub const REQUEST_TIMEOUT: u16 = 408;
    pub const CONFLICT: u16 = 409;
    pub const GONE: u16 = 410;
    pub const LENGTH_REQUIRED: u16 = 411;
    pub const PRECONDITION_FAILED: u16 = 412;
    pub const PAYLOAD_TOO_LARGE: u16 = 413;
    pub const URI_TOO_LONG: u16 = 414;
    pub const UNSUPPORTED_MEDIA_TYPE: u16 = 415;
    pub const REQUESTED_RANGE_NOT_SATISFIABLE: u16 = 416;
    pub const EXPECTATION_FAILED: u16 = 417;
    pub const I_AM_A_TEAPOT: u16 = 418;
    pub const MISDIRECTED: u16 = 421;
    pub const UNPROCESSABLE_ENTITY: u16 = 422;
    pub const FAILED_DEPENDENCY: u16 = 424;
    pub const PRECONDITION_REQUIRED: u16 = 428;
    pub const TOO_MANY_REQUESTS: u16 = 429;
    pub const INTERNAL_SERVER_ERROR: u16 = 500;
    pub const NOT_IMPLEMENTED: u16 = 501;
    pub const BAD_GATEWAY: u16 = 502;
    pub const SERVICE_UNAVAILABLE: u16 = 503;
    pub const GATEWAY_TIMEOUT: u16 = 504;
    pub const HTTP_VERSION_NOT_SUPPORTED: u16 = 505;
}
