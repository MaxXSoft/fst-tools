use std::io::{self, Write};

/// Trait for printing the result.
pub trait Printer {
  fn print<W: Write>(&self, output: &mut W, time: u64, name: &str, value: &[u8]) -> io::Result<()>;
}

/// Prints all informations.
pub struct FullPrinter;

impl Printer for FullPrinter {
  fn print<W: Write>(&self, output: &mut W, time: u64, name: &str, value: &[u8]) -> io::Result<()> {
    write!(output, "#{time} {name} ")?;
    output.write_all(value)?;
    output.write_all(b"\n")
  }
}

/// Prints only variable name.
pub struct NamePrinter;

impl Printer for NamePrinter {
  fn print<W: Write>(&self, output: &mut W, _: u64, name: &str, _: &[u8]) -> io::Result<()> {
    writeln!(output, "{name}")
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  struct BrokenOutput;

  impl Write for BrokenOutput {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
      Err(io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> io::Result<()> {
      Ok(())
    }
  }

  #[test]
  fn printers_propagate_output_errors() {
    assert_eq!(
      FullPrinter
        .print(&mut BrokenOutput, 0, "a", b"1")
        .unwrap_err()
        .kind(),
      io::ErrorKind::BrokenPipe
    );
    assert_eq!(
      NamePrinter
        .print(&mut BrokenOutput, 0, "a", b"1")
        .unwrap_err()
        .kind(),
      io::ErrorKind::BrokenPipe
    );
  }
}
