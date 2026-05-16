//! The local one-shot HTTP callback server.
//!
//! After the user authorises Spottyfi in the browser, Spotify redirects to
//! `http://127.0.0.1:<port>/callback?code=...&state=...`. This module runs a
//! tiny blocking HTTP server that captures exactly one such request, validates
//! the `state` parameter (CSRF protection), shows the user a friendly page and
//! yields the authorization code.

use std::time::Duration;

use tiny_http::{Header, Response, Server};

use crate::error::{AuthError, AuthResult};

/// How long to wait for the user to complete the browser login before the
/// callback server gives up.
pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

/// The HTML shown in the browser once the callback has been received.
const SUCCESS_PAGE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Spottyfi — signed in</title>
<style>
  html,body{height:100%;margin:0}
  body{display:flex;align-items:center;justify-content:center;
       background:#121212;color:#fff;
       font-family:-apple-system,Segoe UI,Roboto,sans-serif}
  .card{text-align:center}
  h1{color:#1ed760;font-size:1.6rem;margin:0 0 .5rem}
  p{color:#b3b3b3}
</style>
</head>
<body>
  <div class="card">
    <h1>You're signed in</h1>
    <p>You can close this tab and return to Spottyfi.</p>
  </div>
</body>
</html>"#;

/// The HTML shown when Spotify returns an error instead of a code.
const ERROR_PAGE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Spottyfi — sign-in failed</title>
<style>
  html,body{height:100%;margin:0}
  body{display:flex;align-items:center;justify-content:center;
       background:#121212;color:#fff;
       font-family:-apple-system,Segoe UI,Roboto,sans-serif}
  .card{text-align:center}
  h1{color:#f15e6c;font-size:1.6rem;margin:0 0 .5rem}
  p{color:#b3b3b3}
</style>
</head>
<body>
  <div class="card">
    <h1>Sign-in failed</h1>
    <p>Return to Spottyfi and try again.</p>
  </div>
</body>
</html>"#;

/// Parse the query string of a `/callback` request path.
///
/// Returns the authorization `code` on success. The `state` parameter is
/// checked against `expected_state` to defend against CSRF. A Spotify-side
/// `error` parameter is surfaced as a [`AuthError::TokenExchange`].
///
/// # Errors
///
/// Returns [`AuthError::StateMismatch`] on a bad `state`, or
/// [`AuthError::CallbackParse`] when required parameters are absent.
pub fn parse_callback_query(path: &str, expected_state: &str) -> AuthResult<String> {
    let query = path.split_once('?').map_or("", |(_, q)| q);

    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    let mut error: Option<String> = None;

    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = url_decode(value);
        match key {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => error = Some(value),
            _ => {}
        }
    }

    if let Some(error) = error {
        return Err(AuthError::TokenExchange(format!(
            "Spotify denied authorization: {error}"
        )));
    }

    match state {
        Some(state) if state == expected_state => {}
        Some(_) => return Err(AuthError::StateMismatch),
        None => {
            return Err(AuthError::CallbackParse(
                "callback was missing the `state` parameter".to_owned(),
            ))
        }
    }

    code.ok_or_else(|| {
        AuthError::CallbackParse("callback was missing the `code` parameter".to_owned())
    })
}

/// Minimal `application/x-www-form-urlencoded` percent-decoder.
///
/// Sufficient for OAuth callback query values, which contain only the
/// authorization code, an opaque `state` and (rarely) an `error` slug.
fn url_decode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'+' => out.push(' '),
            b'%' => {
                let hi = bytes.next();
                let lo = bytes.next();
                match (hi, lo) {
                    (Some(hi), Some(lo)) => {
                        match (hex_value(hi), hex_value(lo)) {
                            (Some(hi), Some(lo)) => out.push(((hi << 4) | lo) as char),
                            // Not valid hex: keep the literal characters.
                            _ => {
                                out.push('%');
                                out.push(hi as char);
                                out.push(lo as char);
                            }
                        }
                    }
                    _ => out.push('%'),
                }
            }
            other => out.push(other as char),
        }
    }
    out
}

/// Decode a single ASCII hex digit.
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Run the one-shot callback server on `127.0.0.1:<port>`.
///
/// This is **blocking** and must be called inside
/// [`tokio::task::spawn_blocking`]. It accepts requests until it sees a
/// `GET /callback`, then responds with [`SUCCESS_PAGE`] (or [`ERROR_PAGE`]) and
/// returns. An overall [`CALLBACK_TIMEOUT`] guards against a login that never
/// completes.
///
/// # Errors
///
/// Returns [`AuthError::CallbackServer`] if the port cannot be bound,
/// [`AuthError::Timeout`] if no callback arrives in time, or whichever error
/// [`parse_callback_query`] produced.
pub fn run_callback_server(port: u16, expected_state: &str) -> AuthResult<String> {
    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr)
        .map_err(|err| AuthError::CallbackServer(format!("could not bind {addr}: {err}")))?;

    let deadline = std::time::Instant::now() + CALLBACK_TIMEOUT;

    loop {
        let remaining = deadline.checked_duration_since(std::time::Instant::now());
        let Some(remaining) = remaining else {
            return Err(AuthError::Timeout);
        };

        let request = match server.recv_timeout(remaining) {
            Ok(Some(request)) => request,
            Ok(None) => return Err(AuthError::Timeout),
            Err(err) => {
                return Err(AuthError::CallbackServer(format!(
                    "callback server receive failed: {err}"
                )))
            }
        };

        let url = request.url().to_owned();

        // Ignore favicon and other stray hits; only `/callback` counts.
        if !url.starts_with("/callback") {
            let _ = request.respond(Response::from_string("Not found").with_status_code(404));
            continue;
        }

        match parse_callback_query(&url, expected_state) {
            Ok(code) => {
                let _ = request.respond(html_response(SUCCESS_PAGE));
                return Ok(code);
            }
            Err(err) => {
                let _ = request.respond(html_response(ERROR_PAGE).with_status_code(400));
                return Err(err);
            }
        }
    }
}

/// Build an HTML `200 OK` response.
fn html_response(body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
        .expect("static header is always valid");
    Response::from_string(body).with_header(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_valid_callback() {
        let code =
            parse_callback_query("/callback?code=abc123&state=xyz", "xyz").expect("should parse");
        assert_eq!(code, "abc123");
    }

    #[test]
    fn rejects_a_state_mismatch() {
        let result = parse_callback_query("/callback?code=abc&state=evil", "xyz");
        assert!(matches!(result, Err(AuthError::StateMismatch)));
    }

    #[test]
    fn rejects_a_missing_state() {
        let result = parse_callback_query("/callback?code=abc", "xyz");
        assert!(matches!(result, Err(AuthError::CallbackParse(_))));
    }

    #[test]
    fn rejects_a_missing_code() {
        let result = parse_callback_query("/callback?state=xyz", "xyz");
        assert!(matches!(result, Err(AuthError::CallbackParse(_))));
    }

    #[test]
    fn surfaces_a_spotify_error() {
        let result = parse_callback_query("/callback?error=access_denied&state=xyz", "xyz");
        assert!(matches!(result, Err(AuthError::TokenExchange(_))));
    }

    #[test]
    fn handles_an_empty_query() {
        let result = parse_callback_query("/callback", "xyz");
        assert!(matches!(result, Err(AuthError::CallbackParse(_))));
    }

    #[test]
    fn url_decodes_percent_escapes() {
        assert_eq!(url_decode("a%2Bb%20c"), "a+b c");
        assert_eq!(url_decode("plain"), "plain");
        assert_eq!(url_decode("a+b"), "a b");
    }

    #[test]
    fn order_of_params_does_not_matter() {
        let code =
            parse_callback_query("/callback?state=xyz&code=abc123", "xyz").expect("should parse");
        assert_eq!(code, "abc123");
    }
}
