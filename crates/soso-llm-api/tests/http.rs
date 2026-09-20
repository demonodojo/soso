//! T11 — parser HTTP incremental y serialización.

use soso_llm_api::{
    format_response, format_sse_response_headers, FeedOutcome, HttpError, HttpReader,
    MAX_BODY_BYTES, MAX_HEADER_BYTES,
};

fn feed_all_at_once(raw: &[u8]) -> soso_llm_api::HttpRequest {
    let mut r = HttpReader::new();
    match r.feed(raw).expect("feed") {
        FeedOutcome::Complete(req) => req,
        FeedOutcome::NeedMore => panic!("incomplete: {raw:?}"),
    }
}

fn feed_fragmented(raw: &[u8], chunk_size: usize) -> Result<FeedOutcome, HttpError> {
    let mut r = HttpReader::new();
    let mut last = FeedOutcome::NeedMore;
    for chunk in raw.chunks(chunk_size.max(1)) {
        last = r.feed(chunk)?;
        if matches!(last, FeedOutcome::Complete(_)) {
            break;
        }
    }
    Ok(last)
}

fn feed_byte_by_byte(raw: &[u8]) -> Result<FeedOutcome, HttpError> {
    feed_fragmented(raw, 1)
}

#[test]
fn get_health_complete() {
    let req = feed_all_at_once(b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/health");
    assert!(req.body.is_empty());
}

#[test]
fn post_with_content_length_case_insensitive() {
    let body = b"{\"x\":1}";
    let mut raw = b"POST /v1/chat/completions HTTP/1.1\r\ncontent-length: 7\r\n\r\n".to_vec();
    raw.extend_from_slice(body);
    let req = feed_all_at_once(&raw);
    assert_eq!(req.body, body);
}

#[test]
fn binary_body_preserved() {
    let body: Vec<u8> = (0u8..=255).collect();
    let mut raw = format!(
        "POST /x HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend_from_slice(&body);
    let req = feed_all_at_once(&raw);
    assert_eq!(req.body, body);
}

#[test]
fn fragmentation_byte_by_byte() {
    let raw = b"POST /sync HTTP/1.1\r\nContent-Length: 3\r\n\r\nabc";
    let outcome = feed_byte_by_byte(raw).unwrap();
    match outcome {
        FeedOutcome::Complete(req) => {
            assert_eq!(req.body, b"abc");
        }
        FeedOutcome::NeedMore => panic!("expected complete"),
    }
}

#[test]
fn fragmentation_arbitrary_splits() {
    let raw = b"GET /v1/models HTTP/1.1\r\nHost: h\r\nAccept: */*\r\n\r\n";
    for split in [1, 2, 3, 7, 11, 17] {
        let outcome = feed_fragmented(raw, split).unwrap();
        assert!(
            matches!(outcome, FeedOutcome::Complete(_)),
            "split {split}"
        );
    }
}

#[test]
fn header_split_mid_name() {
    let raw = b"GET /health HTTP/1.1\r\nContent-Type: application/json\r\n\r\n";
    let outcome = feed_fragmented(raw, 5).unwrap();
    assert!(matches!(outcome, FeedOutcome::Complete(_)));
}

#[test]
fn body_exact_max() {
    let body = vec![b'a'; MAX_BODY_BYTES];
    let mut raw = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY_BYTES
    )
    .into_bytes();
    raw.extend_from_slice(&body);
    let req = feed_all_at_once(&raw);
    assert_eq!(req.body.len(), MAX_BODY_BYTES);
}

#[test]
fn body_over_max_rejected() {
    let cl = MAX_BODY_BYTES + 1;
    let raw = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nContent-Length: {cl}\r\n\r\n"
    );
    let mut r = HttpReader::new();
    let err = r.feed(raw.as_bytes()).unwrap_err();
    assert_eq!(err, HttpError::BodyTooLarge);
}

#[test]
fn headers_at_max_limit() {
    let mut pad = 0usize;
    loop {
        let filler = "x".repeat(pad);
        let raw = format!("GET /health HTTP/1.1\r\nX: {filler}\r\n\r\n");
        match raw.len().cmp(&MAX_HEADER_BYTES) {
            core::cmp::Ordering::Equal => {
                let req = feed_all_at_once(raw.as_bytes());
                assert_eq!(req.path, "/health");
                return;
            }
            core::cmp::Ordering::Less => pad += 1,
            core::cmp::Ordering::Greater => panic!("no exact max header size"),
        }
    }
}

#[test]
fn headers_over_max() {
    let pad = MAX_HEADER_BYTES - 20;
    let filler = "y".repeat(pad);
    let raw = format!("GET /health HTTP/1.1\r\nX: {filler}\r\n\r\n");
    assert!(raw.len() > MAX_HEADER_BYTES);
    let mut r = HttpReader::new();
    let err = r.feed(raw.as_bytes()).unwrap_err();
    assert_eq!(err, HttpError::HeaderTooLarge);
}

#[test]
fn ambiguous_content_length() {
    let raw = b"POST /x HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\na";
    let mut r = HttpReader::new();
    let err = r.feed(raw).unwrap_err();
    assert_eq!(err, HttpError::AmbiguousContentLength);
}

#[test]
fn duplicate_content_length_same_ok() {
    let raw = b"POST /x HTTP/1.1\r\nContent-Length: 2\r\ncontent-length: 2\r\n\r\nhi";
    let req = feed_all_at_once(raw);
    assert_eq!(req.body, b"hi");
}

#[test]
fn rejects_chunked_transfer_encoding() {
    let raw = b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
    let mut r = HttpReader::new();
    let err = r.feed(raw).unwrap_err();
    assert_eq!(err, HttpError::UnsupportedTransferEncoding);
}

#[test]
fn eof_incomplete_body() {
    let raw = b"POST /x HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc";
    let mut r = HttpReader::new();
    assert!(matches!(r.feed(raw).unwrap(), FeedOutcome::NeedMore));
    let err = r.signal_eof().unwrap_err();
    assert_eq!(err, HttpError::IncompleteBody);
}

#[test]
fn eof_valid_zero_length_post() {
    let raw = b"POST /x HTTP/1.1\r\nContent-Length: 0\r\n\r\n";
    let mut r = HttpReader::new();
    r.feed(raw).unwrap();
    // already complete; eof on finished reader is trailing
}

#[test]
fn trailing_bytes_after_request() {
    let raw = b"GET /health HTTP/1.1\r\n\r\nEXTRA";
    let mut r = HttpReader::new();
    let err = r.feed(raw).unwrap_err();
    assert_eq!(err, HttpError::TrailingData);
}

#[test]
fn second_feed_after_complete() {
    let raw = b"GET /health HTTP/1.1\r\n\r\n";
    let mut r = HttpReader::new();
    assert!(matches!(r.feed(raw).unwrap(), FeedOutcome::Complete(_)));
    let err = r.feed(b"x").unwrap_err();
    assert_eq!(err, HttpError::TrailingData);
}

#[test]
fn format_json_response_content_length() {
    let body = br#"{"ok":true}"#;
    let http = format_response(200, &[("Content-Type", "application/json")], body);
    let text = std::str::from_utf8(&http).unwrap();
    assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(text.contains("Content-Length: 11\r\n"));
    assert!(text.contains("Connection: close\r\n"));
    assert!(text.ends_with("{\"ok\":true}"));
}

#[test]
fn format_sse_no_content_length() {
    let hdr = format_sse_response_headers(200);
    let text = std::str::from_utf8(&hdr).unwrap();
    assert!(text.contains("Content-Type: text/event-stream\r\n"));
    assert!(!text.to_ascii_lowercase().contains("content-length:"));
    assert!(text.ends_with("\r\n\r\n"));
}

#[test]
fn exact_body_length_not_accepted_as_complete_early() {
    let raw = b"POST /x HTTP/1.1\r\nContent-Length: 3\r\n\r\nab";
    let mut r = HttpReader::new();
    assert!(matches!(r.feed(raw).unwrap(), FeedOutcome::NeedMore));
}

#[test]
fn body_one_byte_over_content_length() {
    let raw = b"POST /x HTTP/1.1\r\nContent-Length: 2\r\n\r\nabc";
    let mut r = HttpReader::new();
    let err = r.feed(raw).unwrap_err();
    assert_eq!(err, HttpError::TrailingData);
}
