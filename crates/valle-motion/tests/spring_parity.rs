#[path = "support/spring_corpus.rs"]
mod corpus;

#[test]
fn spring_corpus_matches_the_cross_target_golden() {
    assert_eq!(
        hex::encode(corpus::hash()),
        include_str!("golden/spring-v1.sha256").trim()
    );
}
