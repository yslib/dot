use std::borrow::Cow;
use std::io::{self, Write};

use dot::output::{TsvRecord, TsvRenderer};

struct ExampleRow {
    selector: String,
    detail: String,
}

impl TsvRecord for ExampleRow {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        vec![Cow::Borrowed(&self.selector), Cow::Borrowed(&self.detail)]
    }
}

struct OwnedDetailRow<'a> {
    selector: &'a str,
    detail: &'a str,
}

impl TsvRecord for OwnedDetailRow<'_> {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        vec![
            Cow::Borrowed(self.selector),
            Cow::Owned(self.detail.to_owned()),
        ]
    }
}

struct EmptyRow;

impl TsvRecord for EmptyRow {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        Vec::new()
    }
}

struct FailingWriter {
    kind: io::ErrorKind,
}

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(self.kind, "injected writer failure"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn renders_headerless_rows_with_only_later_fields_escaped() {
    let records = [ExampleRow {
        selector: "link:nvim".to_owned(),
        detail: "a\\b\tc\r\nd".to_owned(),
    }];
    let mut output = Vec::new();

    TsvRenderer
        .render(&records, &mut output)
        .expect("TSV should render");

    assert_eq!(output, b"link:nvim\ta\\\\b\\tc\\r\\nd\n");
}

#[test]
fn supports_borrowed_and_owned_fields_without_escaping_the_first_field() {
    let records = [OwnedDetailRow {
        selector: r"target:\dev",
        detail: "owned\tvalue",
    }];
    let mut output = Vec::new();

    TsvRenderer
        .render(&records, &mut output)
        .expect("TSV should render");

    assert_eq!(output, b"target:\\dev\towned\\tvalue\n");
}

#[test]
fn rejects_records_without_fields() {
    let error = TsvRenderer
        .render(&[EmptyRow], &mut Vec::new())
        .expect_err("zero-field records should be rejected");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(
        error.to_string().contains("at least one field"),
        "error should explain the invalid record: {error}"
    );
}

#[test]
fn preserves_broken_pipe_errors_from_the_writer() {
    let records = [ExampleRow {
        selector: "link:nvim".to_owned(),
        detail: "detail".to_owned(),
    }];
    let mut output = FailingWriter {
        kind: io::ErrorKind::BrokenPipe,
    };

    let error = TsvRenderer
        .render(&records, &mut output)
        .expect_err("writer should fail");

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn preserves_other_errors_from_the_writer() {
    let records = [ExampleRow {
        selector: "link:nvim".to_owned(),
        detail: "detail".to_owned(),
    }];
    let mut output = FailingWriter {
        kind: io::ErrorKind::Other,
    };

    let error = TsvRenderer
        .render(&records, &mut output)
        .expect_err("writer should fail");

    assert_eq!(error.kind(), io::ErrorKind::Other);
}
