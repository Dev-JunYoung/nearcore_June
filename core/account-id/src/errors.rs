use std::fmt;
use std::fmt::Write;

//use chrono::{Utc};
/*fn file_write(name:&str) ->std::io::Result<()>{
    let mut file =std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("mylog.txt")
        .expect("Fail");
    // Get the current UTC date and time and format it as a string
    let datetime = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    // Concatenate the date and time with a string
    let message = format!("{} {:?} {} {} \n", datetime,std::thread::current().id(),file!(),name);
    // Write the concatenated string to the file
    std::io::Write::write_all(&mut file, message.as_bytes())?;
    Ok(())
}

fn log(name:&str) {
    let formatted=Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    print!("{} ",formatted);
    println!("my-log : {}|| function : {}", file!(),name);
    println!("-------------------------------------------------------------------------------------");
}*/

/// An error which can be returned when parsing a NEAR Account ID.
#[derive(Eq, Clone, Debug, PartialEq)]
pub struct ParseAccountError {
    pub(crate) kind: ParseErrorKind,
    pub(crate) char: Option<(usize, char)>,
}

impl ParseAccountError {
    /// Returns the specific cause why parsing the Account ID failed.
    pub fn kind(&self) -> &ParseErrorKind {
        &self.kind
    }
}

impl std::error::Error for ParseAccountError {}
impl fmt::Display for ParseAccountError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut buf = self.kind.to_string();
        if let Some((idx, char)) = self.char {
            write!(buf, " {:?} at index {}", char, idx)?
        }
        buf.fmt(f)
    }
}

/// A list of errors that occur when parsing an invalid Account ID.
///
/// Also see [Error kind precedence](crate::AccountId#error-kind-precedence).
#[non_exhaustive]
#[derive(Eq, Clone, Debug, PartialEq)]
pub enum ParseErrorKind {
    /// The Account ID is too long.
    ///
    /// Returned if the `AccountId` is longer than [`AccountId::MAX_LEN`](crate::AccountId::MAX_LEN).
    TooLong,
    /// The Account ID is too short.
    ///
    /// Returned if the `AccountId` is shorter than [`AccountId::MIN_LEN`](crate::AccountId::MIN_LEN).
    TooShort,
    /// The Account ID has a redundant separator.
    ///
    /// This variant would be returned if the Account ID either begins with,
    /// ends with or has separators immediately following each other.
    ///
    /// Cases: `jane.`, `angela__moss`, `tyrell..wellick`
    RedundantSeparator,
    /// The Account ID contains an invalid character.
    ///
    /// This variant would be returned if the Account ID contains an upper-case character, non-separating symbol or space.
    ///
    /// Cases: `ƒelicia.near`, `user@app.com`, `Emily.near`.
    InvalidChar,
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseErrorKind::TooLong => "the Account ID is too long".fmt(f),
            ParseErrorKind::TooShort => "the Account ID is too short".fmt(f),
            ParseErrorKind::RedundantSeparator => "the Account ID has a redundant separator".fmt(f),
            ParseErrorKind::InvalidChar => "the Account ID contains an invalid character".fmt(f),
        }
    }
}
