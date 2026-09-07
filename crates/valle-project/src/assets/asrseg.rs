//! Convert word-level ASR into retrieval segments. Split at long gaps or the maximum segment
//! duration.

use serde_json::{Value, json};

/// Word text, start seconds, and end seconds.
pub type WordTuple = (String, f64, f64);

/// Convert words to items containing `start_ms`, `end_ms`, and `text`.
pub fn words_to_segments(words: &[WordTuple], gap_s: f64, max_len_s: f64) -> Vec<Value> {
    let mut out = Vec::new();
    let mut cur: Vec<&WordTuple> = Vec::new();
    let flush = |cur: &mut Vec<&WordTuple>, out: &mut Vec<Value>| {
        if cur.is_empty() {
            return;
        }
        let start = cur.first().expect("nonempty").1;
        let end = cur.last().expect("nonempty").2;
        let text: String = cur
            .iter()
            .map(|w| w.0.as_str())
            .collect::<Vec<_>>()
            .join("");
        out.push(json!({
            "start_ms": (start * 1000.0).round() as i64,
            "end_ms": (end * 1000.0).round() as i64,
            "text": text,
        }));
        cur.clear();
    };
    for w in words {
        let should_break = match cur.last() {
            Some(last) => w.1 - last.2 > gap_s || w.2 - cur[0].1 > max_len_s,
            None => false,
        };
        if should_break {
            flush(&mut cur, &mut out);
        }
        cur.push(w);
    }
    flush(&mut cur, &mut out);
    out
}

/// Build semantic sentences using terminal punctuation. For overlong unpunctuated text, split at
/// the largest pause. Periods are excluded to avoid splitting decimals and abbreviations.
pub fn sentences(words: &[WordTuple], max_len_s: f64) -> Vec<Value> {
    const ENDERS: &[char] = &['。', '！', '？', '；', '!', '?', ';'];
    let mut out = Vec::new();
    let mut cur: Vec<&WordTuple> = Vec::new();
    let flush = |cur: &mut Vec<&WordTuple>, out: &mut Vec<Value>| {
        if cur.is_empty() {
            return;
        }
        let start = cur.first().expect("nonempty").1;
        let end = cur.last().expect("nonempty").2;
        let text: String = cur.iter().map(|w| w.0.as_str()).collect();
        out.push(json!({
            "start_ms": (start * 1000.0).round() as i64,
            "end_ms": (end * 1000.0).round() as i64,
            "text": text,
        }));
        cur.clear();
    };
    for w in words {
        // Split an overlong sentence at its largest pause. Copy the start time before mutating the
        // buffer.
        let sent_start = cur.first().map(|f| f.1);
        if let Some(start0) = sent_start {
            if w.2 - start0 > max_len_s {
                let mut split_at = 0usize;
                let mut max_gap = -1.0;
                for i in 1..cur.len() {
                    let gap = cur[i].1 - cur[i - 1].2; // Silence between the previous word's end and the next word's start.
                    if gap > max_gap {
                        max_gap = gap;
                        split_at = i;
                    }
                }
                if split_at > 0 {
                    let mut head: Vec<&WordTuple> = cur.drain(..split_at).collect();
                    flush(&mut head, &mut out);
                } else {
                    flush(&mut cur, &mut out);
                }
            }
        }
        let is_end =
            w.0.chars()
                .last()
                .map(|c| ENDERS.contains(&c))
                .unwrap_or(false);
        cur.push(w);
        if is_end {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(t: &str, s: f64, e: f64) -> WordTuple {
        (t.to_owned(), s, e)
    }

    #[test]
    fn splits_on_gap_and_max_len() {
        let words = vec![
            w("今天", 0.0, 0.4),
            w("天气", 0.45, 0.8),
            w("很好", 0.85, 1.2),
            // A 1.5-second pause starts a new segment.
            w("我们", 2.8, 3.1),
            w("出发", 3.15, 3.5),
        ];
        let segs = words_to_segments(&words, 0.8, 15.0);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0]["text"], "今天天气很好");
        assert_eq!(segs[0]["start_ms"], 0);
        assert_eq!(segs[0]["end_ms"], 1200);
        assert_eq!(segs[1]["text"], "我们出发");
        assert_eq!(segs[1]["start_ms"], 2800);

        // Split overlong segments.
        let long: Vec<WordTuple> = (0..40)
            .map(|i| w("字", i as f64 * 0.5, i as f64 * 0.5 + 0.4))
            .collect();
        let segs = words_to_segments(&long, 0.8, 5.0);
        assert!(
            segs.len() > 1,
            "overlong segments must split: {}",
            segs.len()
        );
    }
}
