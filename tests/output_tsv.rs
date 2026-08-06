use std::borrow::Cow;
use std::cell::Cell;
use std::io::{self, Write};

use dot_core::output::{TsvRecord, TsvRenderer};

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

struct CountingRow<'a> {
    calls: &'a Cell<usize>,
}

impl TsvRecord for CountingRow<'_> {
    fn fields(&self) -> Vec<Cow<'_, str>> {
        self.calls.set(self.calls.get() + 1);
        vec![Cow::Borrowed("selector"), Cow::Borrowed("detail")]
    }
}

#[derive(Default)]
struct TouchTrackingWriter {
    touched: bool,
}

impl Write for TouchTrackingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.touched = true;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.touched = true;
        Ok(())
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

struct PartialThenFailWriter {
    accepted: Vec<u8>,
    retried_with: Option<Vec<u8>>,
    kind: io::ErrorKind,
}

impl Write for PartialThenFailWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.accepted.is_empty() {
            let prefix_length = buffer.len().min(4);
            self.accepted.extend_from_slice(&buffer[..prefix_length]);
            return Ok(prefix_length);
        }

        self.retried_with = Some(buffer.to_vec());
        Err(io::Error::new(self.kind, "failure after partial write"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn renders_an_empty_record_slice_as_empty_output() {
    let records: [ExampleRow; 0] = [];
    let mut output = Vec::new();

    TsvRenderer
        .render(&records, &mut output)
        .expect("empty input should render");

    assert!(output.is_empty());
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
fn preserves_multibyte_utf8_adjacent_to_every_escaped_byte() {
    let records = [ExampleRow {
        selector: "selector".to_owned(),
        detail: "é\\中\t🙂\rø\nß".to_owned(),
    }];
    let mut output = Vec::new();

    TsvRenderer
        .render(&records, &mut output)
        .expect("TSV should render");

    assert_eq!(output, "selector\té\\\\中\\t🙂\\rø\\nß\n".as_bytes());
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
fn preparation_materializes_fields_before_rendering() {
    let calls = Cell::new(0);
    let records = [CountingRow { calls: &calls }];

    let prepared = TsvRenderer
        .prepare(&records)
        .expect("valid records should prepare");
    assert_eq!(calls.get(), 1);

    let mut output = Vec::new();
    prepared
        .render(&mut output)
        .expect("prepared rows should render");

    assert_eq!(calls.get(), 1, "rendering must not revisit domain records");
    assert_eq!(output, b"selector\tdetail\n");
}

#[test]
fn zero_field_validation_happens_before_a_writer_is_touched() {
    let mut output = TouchTrackingWriter::default();

    let error = TsvRenderer
        .render(&[EmptyRow], &mut output)
        .expect_err("empty rows should fail during preparation");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(!output.touched, "record validation must precede output");
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

#[test]
fn retries_after_a_partial_write_and_preserves_the_eventual_error() {
    let records = [ExampleRow {
        selector: "link:nvim".to_owned(),
        detail: "detail".to_owned(),
    }];
    let mut output = PartialThenFailWriter {
        accepted: Vec::new(),
        retried_with: None,
        kind: io::ErrorKind::PermissionDenied,
    };

    let error = TsvRenderer
        .render(&records, &mut output)
        .expect_err("writer should fail after accepting a prefix");

    assert_eq!(output.accepted, b"link");
    assert_eq!(output.retried_with.as_deref(), Some(b":nvim".as_slice()));
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}
