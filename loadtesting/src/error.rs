use std::error::Error as StdError;
use std::fmt::Display;

pub type Error = Box<dyn StdError + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

pub fn error(message: impl Into<String>) -> Error {
    std::io::Error::other(message.into()).into()
}

pub trait ResultContext<T> {
    fn context(self, context: impl Display) -> Result<T>;
}

impl<T, E> ResultContext<T> for std::result::Result<T, E>
where
    E: Display,
{
    fn context(self, context: impl Display) -> Result<T> {
        self.map_err(|source| error(format!("{context}: {source}")))
    }
}
