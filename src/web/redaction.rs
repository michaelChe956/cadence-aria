const SENSITIVE_MARKERS: [&str; 5] = [
    "Authorization:",
    "api_key",
    "API_KEY",
    "token",
    "private_key",
];

pub fn redact_sensitive_lines(input: &str) -> String {
    let mut redacted = input
        .lines()
        .map(|line| {
            if SENSITIVE_MARKERS.iter().any(|marker| line.contains(marker)) {
                "[REDACTED]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if input.ends_with('\n') {
        redacted.push('\n');
    }
    redacted
}
