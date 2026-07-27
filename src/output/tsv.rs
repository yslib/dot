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
        for record in records {
            let fields = record.fields();
            let Some((first, remaining)) = fields.split_first() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "TSV records must contain at least one field",
                ));
            };

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
