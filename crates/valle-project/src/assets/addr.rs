//! Address assets by unique hash prefixes; report candidates for ambiguous prefixes.

use crate::ContentDigest;
use crate::assets::home::Home;
use crate::assets::report::{AssetsError, Result};

/// Resolve a unique prefix to a full hash. Full hashes use direct lookup; other prefixes scan
/// metadata and report missing or ambiguous matches.
pub fn find_hash(home: &Home, prefix: &str) -> Result<String> {
    let p = if prefix.starts_with("sha256:") {
        ContentDigest::parse(prefix)
            .map_err(|_| {
                AssetsError::not_found(format!("invalid content digest: '{prefix}'")).with_hint(
                    "a full digest must be sha256: followed by 64 lowercase hexadecimal characters",
                )
            })?
            .as_hex()
    } else {
        prefix.to_ascii_lowercase()
    };
    if p.len() < 3 {
        return Err(
            AssetsError::not_found(format!("prefix is too short: '{prefix}'"))
                .with_hint("provide at least three hexadecimal prefix characters"),
        );
    }
    if !p.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(
            AssetsError::not_found(format!("invalid hash prefix: '{prefix}'"))
                .with_hint("hashes use hexadecimal characters"),
        );
    }
    if p.len() == 64 {
        if home.meta_path(&p).exists() {
            return Ok(p);
        }
        return Err(AssetsError::not_found(format!("asset not found: {prefix}")));
    }

    // Use a single shard when at least two prefix characters are available.
    let meta_root = home.meta_dir();
    let (shard, rest) = p.split_at(2);
    let mut matches = Vec::new();
    let dir = meta_root.join(shard);
    if dir.exists() {
        for e in std::fs::read_dir(&dir)?.filter_map(|e| e.ok()) {
            let name = e.file_name();
            let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".json")) else {
                continue;
            };
            if stem.starts_with(rest) {
                matches.push(format!("{shard}{stem}"));
            }
        }
    }
    match matches.len() {
        0 => Err(
            AssetsError::not_found(format!("no asset matches prefix '{prefix}'"))
                .with_hint("list registered assets with `valle assets list`"),
        ),
        1 => Ok(matches.pop().expect("len==1")),
        _ => {
            matches.sort();
            let shown: Vec<String> = matches.iter().take(5).map(|h| h[..12].to_owned()).collect();
            Err(AssetsError::not_found(format!(
                "prefix '{prefix}' matches {} assets and is ambiguous",
                matches.len()
            ))
            .with_hint(format!(
                "use a longer prefix; candidates: {}",
                shown.join(", ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_meta(home: &Home, hash: &str) {
        let p = home.meta_path(hash);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"{}").unwrap();
    }

    #[test]
    fn prefix_addressing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::at(tmp.path());
        let h1 = format!("abcd11{}", "0".repeat(58));
        let h2 = format!("abcd22{}", "0".repeat(58));
        let h3 = format!("ff00{}", "1".repeat(60));
        for h in [&h1, &h2, &h3] {
            fake_meta(&home, h);
        }
        // Resolve a unique prefix.
        assert_eq!(find_hash(&home, "abcd11").unwrap(), h1);
        assert_eq!(find_hash(&home, "ff0").unwrap(), h3);
        // Look up a full hash directly.
        assert_eq!(find_hash(&home, &h2).unwrap(), h2);
        assert_eq!(
            find_hash(&home, &format!("sha256:{h2}")).unwrap(),
            h2,
            "canonical ContentDigest output must compose with asset commands"
        );
        // Ambiguous prefixes report candidates.
        let err = find_hash(&home, "abcd").unwrap_err();
        assert!(err.hint.unwrap().contains("abcd11"));
        // Reject missing, short, and invalid prefixes.
        assert!(find_hash(&home, "beef").is_err());
        assert!(find_hash(&home, "ab").is_err());
        assert!(find_hash(&home, "xyz!").is_err());
        assert!(find_hash(&home, &format!("sha256:{}", "A".repeat(64))).is_err());
    }
}
