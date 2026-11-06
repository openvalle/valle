//! Studio and the SDK, from a development build or the package's Brotli-compressed bytes.

use std::path::Path;

use crate::http::{self, Response};

#[cfg(feature = "embed-web")]
const EMBEDDED: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/web.rs"));

pub(super) fn serve(web: Option<&Path>, path: &str, encoding: Option<&str>) -> Response {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let Some(path) = valle_project::inside(path) else {
        return Response::new(403, "text/plain", "not in Studio");
    };
    if let Some(web) = web {
        return file(web, &path);
    }
    #[cfg(feature = "embed-web")]
    {
        embedded(&path, encoding)
    }
    #[cfg(not(feature = "embed-web"))]
    {
        let _ = encoding;
        file(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web"),
            &path,
        )
    }
}

fn file(web: &Path, path: &str) -> Response {
    let (dir, name) = match path.strip_prefix("valle/") {
        Some(rest) => ("packages/engine/dist", rest),
        None => ("apps/studio/dist", path),
    };
    match std::fs::read(web.join(dir).join(name)) {
        Ok(bytes) => Response::new(200, http::kind(path), bytes),
        Err(_) if path == "index.html" => Response::new(
            404,
            "text/plain",
            format!(
                "Studio is not built in {}: run cargo xtask build-web",
                web.display()
            ),
        ),
        Err(_) => Response::new(404, "text/plain", "not found"),
    }
}

#[cfg(feature = "embed-web")]
fn embedded(path: &str, encoding: Option<&str>) -> Response {
    let Some((_, bytes)) = EMBEDDED.iter().find(|(name, _)| *name == path) else {
        return Response::new(404, "text/plain", "not found");
    };
    let response = if accepts(encoding, "br") {
        Response::new(200, http::kind(path), *bytes).header("content-encoding", "br")
    } else if accepts(encoding, "identity") {
        let mut plain = Vec::new();
        let mut compressed = *bytes;
        match brotli_decompressor::BrotliDecompress(&mut compressed, &mut plain) {
            Ok(()) => Response::new(200, http::kind(path), plain),
            Err(_) => Response::new(500, "text/plain", "could not decode the packaged asset"),
        }
    } else {
        Response::new(406, "text/plain", "accept br or identity to read Studio")
    };
    response.header("vary", "Accept-Encoding")
}

/// Explicit exclusions override `*`; identity is allowed unless it or all encodings are
/// refused. Without negotiation, send plain bytes (including to command-line HTTP clients).
#[cfg(any(feature = "embed-web", test))]
fn accepts(header: Option<&str>, coding: &str) -> bool {
    let (mut explicit, mut wildcard) = (None, None);
    for part in header.unwrap_or("").split(',') {
        let mut parts = part.trim().split(';');
        let name = parts.next().unwrap_or("").trim();
        let weight = parts.find_map(|p| {
            let (key, value) = p.trim().split_once('=')?;
            key.trim().eq_ignore_ascii_case("q").then(|| value.trim())
        });
        let allowed = weight.is_none_or(|q| q.parse::<f32>().is_ok_and(|q| q > 0.0 && q <= 1.0));
        if name.eq_ignore_ascii_case(coding) {
            explicit = Some(allowed);
        } else if name == "*" {
            wildcard = Some(allowed);
        }
    }
    explicit.or(wildcard).unwrap_or(coding == "identity")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_negotiation_observes_exclusions() {
        for header in [None, Some("gzip"), Some("br;q=0, *;q=1"), Some("br;q=oops")] {
            assert!(!accepts(header, "br"), "{header:?}");
            assert!(accepts(header, "identity"), "{header:?}");
        }
        for header in ["gzip, br", "BR; q=0.5", "*;q=1"] {
            assert!(accepts(Some(header), "br"), "{header}");
        }
        assert!(!accepts(Some("br, identity;q=0"), "identity"));
        assert!(!accepts(Some("*;q=0"), "identity"));
        assert!(accepts(Some("*;q=0, identity;q=1"), "identity"));
    }

    #[test]
    fn assets_stay_inside_the_web_build() {
        for path in [
            "/../Cargo.toml",
            "/valle/../../Cargo.toml",
            "/C:/outside.js",
        ] {
            assert_eq!(serve(None, path, None).status, 403, "{path}");
        }
    }

    #[test]
    fn an_explicit_build_overrides_embedded_assets_without_falling_back() {
        let root = std::env::temp_dir().join(format!("valle-web-override-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("apps/studio/dist")).unwrap();
        std::fs::write(root.join("apps/studio/dist/index.html"), b"development").unwrap();
        assert_eq!(serve(Some(&root), "/", Some("br")).body, b"development");
        assert_eq!(serve(Some(&root), "/main.js", None).status, 404);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "embed-web")]
    #[test]
    fn every_packaged_asset_decodes_to_the_web_build() {
        let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web");
        assert!(EMBEDDED.iter().any(|(path, _)| *path == "index.html"));
        assert!(
            EMBEDDED
                .iter()
                .any(|(path, _)| *path == "valle/valle_web_bg.wasm")
        );
        for (path, bytes) in EMBEDDED {
            let plain = serve(None, path, None);
            assert_eq!(plain.status, 200, "{path}");
            assert_eq!(plain.body, file(&web, path).body, "{path}");
            let br = serve(None, path, Some("gzip, br"));
            assert_eq!(br.body, *bytes, "{path}");
            assert!(
                br.headers
                    .iter()
                    .any(|(k, v)| k == "content-encoding" && v == "br")
            );
            assert_eq!(serve(None, path, Some("br;q=0")).body, plain.body);
        }
        assert_eq!(serve(None, "/", Some("*;q=0")).status, 406);
        assert_eq!(serve(None, "/missing.js", Some("br")).status, 404);
    }
}
