use std::borrow::Cow;
use std::io::{self, Write};

pub trait TsvRecord {
    /// Returns at least one field, with a canonical TSV-safe selector first.
    ///
    /// The renderer emits the first field verbatim and escapes backslash, tab,
    /// carriage return, and line feed only in later fields.
    fn fields(&self) -> Vec<Cow<'_, str>>;
}

pub struct TsvRenderer;

impl TsvRenderer {
    /// Materializes and validates every record field before output begins.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] when any record has no fields.
    pub fn prepare<'a, R: TsvRecord>(&self, records: &'a [R]) -> io::Result<PreparedTsv<'a>> {
        let mut rows = Vec::with_capacity(records.len());
        for record in records {
            let fields = record.fields();
            if fields.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "TSV records must contain at least one field",
                ));
            }
            rows.push(fields);
        }
        Ok(PreparedTsv { rows })
    }

    /// Renders headerless records with one newline after each record.
    ///
    /// The first field is emitted verbatim; later fields escape backslash, tab,
    /// carriage return, and line feed.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] for a record without fields and
    /// propagates writer errors unchanged.
    pub fn render<R: TsvRecord>(&self, records: &[R], output: &mut dyn Write) -> io::Result<()> {
        self.prepare(records)?.render(output)
    }

    /// Renders records into an owned UTF-8 string.
    pub fn render_to_string<R: TsvRecord>(&self, records: &[R]) -> io::Result<String> {
        let mut output = Vec::new();
        self.render(records, &mut output)?;
        Ok(String::from_utf8(output).expect("TSV fields and escapes are always UTF-8"))
    }
}

/// A complete validated set of TSV fields ready for output.
pub struct PreparedTsv<'a> {
    rows: Vec<Vec<Cow<'a, str>>>,
}

impl PreparedTsv<'_> {
    /// Escapes and writes the prepared rows without revisiting domain records.
    ///
    /// # Errors
    ///
    /// Propagates writer errors unchanged.
    pub fn render(&self, output: &mut dyn Write) -> io::Result<()> {
        for fields in &self.rows {
            let (first, remaining) = fields
                .split_first()
                .expect("preparation rejects records without fields");
            output.write_all(first.as_bytes())?;
            for field in remaining {
                output.write_all(b"\t")?;
                write_escaped(field, output)?;
            }
            output.write_all(b"\n")?;
        }

        Ok(())
    }
}

fn write_escaped(value: &str, output: &mut dyn Write) -> io::Result<()> {
    let bytes = value.as_bytes();
    let mut literal_start = 0;

    for (index, byte) in bytes.iter().enumerate() {
        let replacement: &[u8] = match byte {
            b'\\' => b"\\\\",
            b'\t' => b"\\t",
            b'\r' => b"\\r",
            b'\n' => b"\\n",
            _ => continue,
        };
        output.write_all(&bytes[literal_start..index])?;
        output.write_all(replacement)?;
        literal_start = index + 1;
    }

    output.write_all(&bytes[literal_start..])
}
