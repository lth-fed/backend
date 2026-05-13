use std::{borrow::Cow, fmt, str::FromStr};

use poem_openapi::{
    registry::{MetaSchema, MetaSchemaRef},
    types::ParseResult,
};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use sqlx::postgres::types::{PgLTree, PgLTreeLabel};

/// Represents a group path as a string, parsed into a [`PgLTree`] internally.
#[derive(sqlx::Type, Debug, Default)]
#[sqlx(transparent)]
#[derive(DeserializeFromStr, SerializeDisplay)]
pub struct Path(pub PgLTree);

impl Path {
    /// Joins a [`PgLTreeLabel`] to the end of this path, returning a new [`Path`].
    #[must_use]
    pub fn join(&self, label: PgLTreeLabel) -> Self {
        let mut path = self.0.clone();
        path.push(label);
        Self(path)
    }

    /// Returns the parent path of this path by removing the last label.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        let mut path = self.0.clone();
        path.pop().is_some().then_some(Self(path))
    }
}

impl From<PgLTree> for Path {
    fn from(value: PgLTree) -> Self {
        Self(value)
    }
}

// FromStr is just forwarded to PgLTree.
impl FromStr for Path {
    type Err = <PgLTree as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let tree = PgLTree::from_str(s)?;
        Ok(Self(tree))
    }
}

// Display is just forwarded to PgLTree.
impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// This monstrosity makes GroupPath usable with poem-openapi.
impl poem_openapi::types::Type for Path {
    const IS_REQUIRED: bool = true;

    type RawValueType = String;
    type RawElementValueType = String;

    fn name() -> Cow<'static, str> {
        "string_group_path".into()
    }

    fn schema_ref() -> MetaSchemaRef {
        MetaSchemaRef::Inline(Box::new(MetaSchema::new_with_format(
            "string",
            "group_path",
        )))
    }

    fn as_raw_value(&self) -> Option<&Self::RawValueType> {
        None
    }

    fn raw_element_iter<'a>(
        &'a self,
    ) -> Box<dyn Iterator<Item = &'a Self::RawElementValueType> + 'a> {
        Box::new(vec![].into_iter())
    }
}

impl poem_openapi::types::ParseFromJSON for Path {
    fn parse_from_json(value: Option<serde_json::Value>) -> ParseResult<Self> {
        let value = value.unwrap_or_default(); // default is Value::Null
        serde_json::from_value(value).map_err(Into::into)
    }
}

impl poem_openapi::types::ToJSON for Path {
    fn to_json(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self).ok()
    }
}

impl poem_openapi::types::ParseFromParameter for Path {
    fn parse_from_parameter(value: &str) -> ParseResult<Self> {
        value.parse().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use sqlx::postgres::types::PgLTreeLabel;

    use crate::group::path::Path;

    #[test]
    fn default_is_empty() {
        assert_eq!(Path::default().to_string(), "");
        assert_eq!(
            Path::default()
                .join(PgLTreeLabel::new("tlth").unwrap())
                .to_string(),
            "tlth"
        );
    }
}
