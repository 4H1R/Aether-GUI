// A real child process for PTY integration tests; no network or credentials.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert_eq!(&args[..2], ["--masque", "--h3"]);
    assert!(args.windows(2).any(|a| a == ["--bind", "127.0.0.1:1819"]));
    assert!(!args.iter().any(|a| a.contains("fixture-token")));
    assert_eq!(
        std::env::var("AETHER_ACCESS_TOKEN").unwrap(),
        "fixture-token"
    );
    assert!(std::env::var("AETHER_UNEXPECTED_SETTING").is_err());
    println!("fixture ready: fixture-token");
}
