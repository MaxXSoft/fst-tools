/// Trait for printing the result.
pub trait Printer {
  fn print(&self, time: u64, name: &str, value: &[u8]);
}

/// Prints all informations.
pub struct FullPrinter;

impl Printer for FullPrinter {
  fn print(&self, time: u64, name: &str, value: &[u8]) {
    print!("#{time} {name} ");
    for v in value {
      print!("{}", *v as char);
    }
    println!();
  }
}

/// Prints only variable name.
pub struct NamePrinter;

impl Printer for NamePrinter {
  fn print(&self, _: u64, name: &str, _: &[u8]) {
    println!("{name}")
  }
}
