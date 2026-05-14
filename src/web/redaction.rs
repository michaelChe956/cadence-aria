const SENSITIVE_MARKERS: [&str; 8] = [
    "authorization",
    "api_key",
    "api-key",
    "apikey",
    "token",
    "private_key",
    "private-key",
    "privatekey",
];

pub fn redact_sensitive_lines(input: &str) -> String {
    let mut redacted = String::with_capacity(input.len());
    for segment in input.split_inclusive('\n') {
        let (line, line_ending) = split_line_ending(segment);
        if contains_sensitive_marker(line) {
            redacted.push_str("[REDACTED]");
            redacted.push_str(line_ending);
        } else {
            redacted.push_str(segment);
        }
    }
    redacted
}

fn split_line_ending(segment: &str) -> (&str, &str) {
    if let Some(without_lf) = segment.strip_suffix('\n') {
        if let Some(without_crlf) = without_lf.strip_suffix('\r') {
            return (without_crlf, "\r\n");
        }
        return (without_lf, "\n");
    }
    (segment, "")
}

fn contains_sensitive_marker(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    SENSITIVE_MARKERS.iter().any(|marker| line.contains(marker))
}
