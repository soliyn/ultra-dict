//! The `dict` URI scheme: serves dictionary resources (CSS, images, fonts,
//! sounds, …) to the article frames from [`Library::resource`].
//!
//! A resource URL is `<base><dict-id>/<path>`, each path segment
//! percent-encoded. The base depends on the webview: `dict://localhost/` on
//! macOS and Linux, `http://dict.localhost/` on Windows, where WebView2 only
//! routes custom schemes in that form. The frontend gets it from Tauri's
//! `convertFileSrc("", "dict")` (see `src/resources.ts`).
//!
//! The dictionary id is the first path segment, not the host, because the
//! Windows host is fixed. So a root-relative reference (`/img/a.png`) can't
//! resolve against a resource URL by itself; the frontend rewrites article
//! HTML to absolute resource URLs instead.

use std::borrow::Cow;
use std::sync::PoisonError;

use percent_encoding::percent_decode_str;
use tauri::http::{Response, StatusCode, header};
use tauri::{Builder, Manager, Runtime};

use crate::commands::SharedLibrary;
use crate::library::{DictionaryId, Library};

/// The scheme name.
pub const SCHEME: &str = "dict";

/// Registers the scheme on `builder`. The app's state must include a
/// [`SharedLibrary`].
pub fn register<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.register_asynchronous_uri_scheme_protocol(SCHEME, |ctx, request, responder| {
        let app = ctx.app_handle().clone();
        let path = request.uri().path().to_owned();
        // Off the webview's thread: the library may be busy indexing.
        tauri::async_runtime::spawn_blocking(move || {
            let library = app.state::<SharedLibrary>();
            let library = library.lock().unwrap_or_else(PoisonError::into_inner);
            responder.respond(respond(&library, &path));
        });
    })
}

/// The response for the resource URL whose path is `uri_path`
/// (`/<dict-id>/<percent-encoded path>`).
///
/// A missing resource, an unknown dictionary, a malformed URL and a refused
/// path all give an empty `404`, so an article renders without whatever is
/// missing. Only a library (database) failure gives `500`.
pub fn respond(library: &Library, uri_path: &str) -> Response<Vec<u8>> {
    let Some((id, path)) = parse(uri_path) else {
        return reply(StatusCode::NOT_FOUND, None, Vec::new());
    };
    match library.resource(id, &path) {
        Ok(Some(resource)) => reply(StatusCode::OK, Some(resource.content_type), resource.bytes),
        Ok(None) => reply(StatusCode::NOT_FOUND, None, Vec::new()),
        Err(error) => reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some("text/plain; charset=utf-8"),
            error.to_string().into_bytes(),
        ),
    }
}

fn parse(uri_path: &str) -> Option<(DictionaryId, Cow<'_, str>)> {
    let (id, path) = uri_path.trim_start_matches('/').split_once('/')?;
    let id = DictionaryId::from_raw(id.parse().ok()?);
    Some((id, percent_decode_str(path).decode_utf8().ok()?))
}

fn reply(status: StatusCode, content_type: Option<&str>, body: Vec<u8>) -> Response<Vec<u8>> {
    let mut response = Response::builder()
        .status(status)
        // Fonts are fetched with CORS, and article frames have another
        // origin (an opaque one once scripts are allowed, #12).
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    if let Some(content_type) = content_type {
        response = response.header(header::CONTENT_TYPE, content_type);
    }
    response.body(body).expect("static headers are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_split_into_id_and_decoded_path() {
        let (id, path) = parse("/3/img/a%20b.png").unwrap();
        assert_eq!((id, path.as_ref()), (DictionaryId::from_raw(3), "img/a b.png"));
        assert_eq!(parse("/3/%E2%82%AC.css").unwrap().1, "€.css");
        assert_eq!(parse("/-1/x").unwrap().0, DictionaryId::from_raw(-1));
    }

    #[test]
    fn malformed_urls_do_not_parse() {
        for path in ["", "/", "/3", "/x/a.css", "/3/%FF.css"] {
            assert!(parse(path).is_none(), "{path:?}");
        }
    }
}
