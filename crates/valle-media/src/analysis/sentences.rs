use serde::{Deserialize, Serialize};

use super::Word;

/// A sentence-level projection of the word-timestamp source of truth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sentence {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// Project a word stream into human-readable sentences.
///
/// Sentence-ending punctuation is authoritative. If a sentence exceeds `max_len_s` without such
/// punctuation, the largest internal pause is used as a fallback split point.
pub fn sentences(words: &[Word], max_len_s: f64) -> Vec<Sentence> {
    const ENDERS: &[char] = &['。', '！', '？', '；', '!', '?', ';'];

    let mut out = Vec::new();
    let mut current: Vec<&Word> = Vec::new();
    let flush = |current: &mut Vec<&Word>, out: &mut Vec<Sentence>| {
        if current.is_empty() {
            return;
        }
        out.push(Sentence {
            text: current.iter().map(|word| word.text.as_str()).collect(),
            start: current.first().expect("non-empty sentence").start,
            end: current.last().expect("non-empty sentence").end,
        });
        current.clear();
    };

    for word in words {
        if let Some(start) = current.first().map(|first| first.start)
            && word.end - start > max_len_s
        {
            let mut split_at = 0;
            let mut largest_gap = -1.0;
            for index in 1..current.len() {
                let gap = current[index].start - current[index - 1].end;
                if gap > largest_gap {
                    largest_gap = gap;
                    split_at = index;
                }
            }
            if split_at > 0 {
                let mut head: Vec<&Word> = current.drain(..split_at).collect();
                flush(&mut head, &mut out);
            } else {
                flush(&mut current, &mut out);
            }
        }

        let ends_sentence = word
            .text
            .chars()
            .last()
            .is_some_and(|character| ENDERS.contains(&character));
        current.push(word);
        if ends_sentence {
            flush(&mut current, &mut out);
        }
    }
    flush(&mut current, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(id: &str, text: &str, start: f64, end: f64) -> Word {
        Word {
            id: id.to_owned(),
            text: text.to_owned(),
            start,
            end,
        }
    }

    #[test]
    fn punctuation_ends_a_sentence() {
        let projected = sentences(
            &[word("w0", "你好。", 0.0, 0.4), word("w1", "世界", 0.5, 0.9)],
            15.0,
        );
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].text, "你好。");
        assert_eq!(projected[1].text, "世界");
    }

    #[test]
    fn overlong_sentence_splits_at_largest_pause() {
        let projected = sentences(
            &[
                word("w0", "a", 0.0, 1.0),
                word("w1", "b", 1.1, 2.0),
                word("w2", "c", 5.0, 6.0),
                word("w3", "d", 10.0, 11.0),
            ],
            8.0,
        );
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].text, "ab");
        assert_eq!(projected[1].text, "cd");
    }
}
