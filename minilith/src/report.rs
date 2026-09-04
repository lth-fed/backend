use std::fmt::Debug;

use minilith_errors::{AlertLevel, MinilithErrorResultExt as _, MinilithResult, alert};
use poem_openapi::Enum;
use serde::Serialize;
use tracing::error;
use tracing::{info_span, warn};
use typst::diag::{FileError, FileResult, PackageError};
use typst::foundations::Bytes;
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt as _};
use typst_layout::PagedDocument;

const TYPST_DOC: &str = include_str!("./report.typ");

#[derive(Serialize, Debug, Clone, Copy, Enum)]
pub enum Language {
    #[serde(rename = "sv")]
    #[oai(rename = "sv")]
    Swedish,
    #[serde(rename = "en")]
    #[oai(rename = "en")]
    English,
}
#[derive(Serialize, Debug, Clone, Copy, Enum, PartialEq, Eq, PartialOrd, Ord)]
#[oai(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Ticket,
    Option,
    External,
}
#[derive(Serialize, Debug, Clone, Copy, Enum)]
#[oai(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ImageKind {
    Svg,
    Png,
    /// map jpeg to this.
    Jpg,
    Webp,
}

#[derive(Serialize, Debug, Clone)]
pub struct Object {
    pub name: String,
    pub kind: Kind,
    pub price: i64,
    pub number: i64,
}
#[derive(Serialize, Debug, Clone)]
pub struct AlcoholCategory {
    pub name: String,
    pub amount: i64,
}
#[derive(Serialize, Debug, Clone)]
pub struct Data {
    pub language: Language,
    pub activity_name: String,
    pub creator_name: String,
    pub creator_logo_format: Option<ImageKind>,
    #[serde(skip)]
    #[allow(clippy::struct_field_names, reason = "stfu")]
    pub creator_logo_data: Option<bytes::Bytes>,
    pub fees: i64,
    pub fees_external: i64,
    pub per_object: Vec<Object>,
    pub per_alcohol_category: Vec<AlcoholCategory>,
    pub receipt_count: usize,
    #[serde(skip)]
    pub receipts: Vec<bytes::Bytes>,
}

pub(crate) struct OurWonderfulTypstWorldBase {
    lib: LazyHash<Library>,
    fonts: typst_kit::fonts::FontStore,
}
impl Debug for OurWonderfulTypstWorldBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<Opaque>")
    }
}
impl Default for OurWonderfulTypstWorldBase {
    fn default() -> Self {
        let mut fonts = typst_kit::fonts::FontStore::default();
        fonts.extend(typst_kit::fonts::embedded());

        Self {
            lib: Library::default().into(),
            fonts,
        }
    }
}
struct OurWonderfulTypstWorld<'a> {
    inner: &'a OurWonderfulTypstWorldBase,
    now: typst_kit::datetime::Time,

    data: &'a Data,
}
impl typst::World for OurWonderfulTypstWorld<'_> {
    fn library(&self) -> &LazyHash<Library> {
        &self.inner.lib
    }

    fn book(&self) -> &LazyHash<FontBook> {
        self.inner.fonts.book()
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.inner.fonts.font(index)
    }

    #[allow(
        clippy::unwrap_used,
        reason = "we can't error handle here + it's guaranteed not to panic"
    )]
    fn main(&self) -> FileId {
        FileId::new(RootedPath::new(
            VirtualRoot::Project,
            VirtualPath::new("receipt.typ").unwrap(),
        ))
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if *id.root() != VirtualRoot::Project {
            return Err(FileError::Package(PackageError::Other(None)));
        }
        let data = match id.vpath().get_without_slash() {
            "receipt.typ" => Source::new(id, TYPST_DOC.to_owned()),
            "data.json" | "image" => return Err(FileError::NotSource),
            _ => return Err(FileError::NotFound(id.vpath().get_without_slash().into())),
        };
        Ok(data)
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if *id.root() != VirtualRoot::Project {
            return Err(FileError::Package(PackageError::Other(None)));
        }

        let path = id.vpath().get_without_slash();
        let data = match path {
            "receipt.typ" => Bytes::new(TYPST_DOC),
            "data.json" => Bytes::new(serde_json::to_string(&self.data).map_err(|error| {
                alert(AlertLevel::L2, "Failed to serialize data.json for receipt.");
                error!(?error, "Failed to serialize data.json for receipt.");
                FileError::Other(None)
            })?),
            "image" if let Some(data) = &self.data.creator_logo_data => {
                Bytes::new(bytes::Bytes::clone(data))
            }
            _ => {
                let receipt = path
                    .strip_prefix("transaction-receipt-")
                    .and_then(|path| path.strip_suffix(".pdf"))
                    .and_then(|index| index.parse::<usize>().ok())
                    .and_then(|index| self.data.receipts.get(index))
                    .ok_or_else(|| FileError::NotFound(path.into()))?;
                Bytes::new(bytes::Bytes::clone(receipt))
            }
        };
        Ok(data)
    }

    fn today(
        &self,
        offset: Option<typst::foundations::Duration>,
    ) -> Option<typst::foundations::Datetime> {
        self.now.today(offset)
    }
}

/// You have to assure data is not absurd (i.e. VERY long).
///
/// # Returns
///
/// PDF.
///
/// # Errors
///
/// Errors if the typst compilation fails (which should never happen; we control the typst
/// document). Though could happen if image is shit.
pub fn compile(world: &OurWonderfulTypstWorldBase, data: &Data) -> MinilithResult<Vec<u8>> {
    let _span = info_span!("typst receipt compilation").entered();

    let mut fonts = typst_kit::fonts::FontStore::default();
    fonts.extend(typst_kit::fonts::embedded());

    let world = OurWonderfulTypstWorld {
        inner: world,
        now: typst_kit::datetime::Time::system(),
        data,
    };

    let output = typst::compile::<PagedDocument>(&world);
    if !output.warnings.is_empty() {
        for warning in &output.warnings {
            warn!(
                "Typst warning: {} (hints {:?})",
                warning.message, warning.hints
            );
        }
    }
    let doc = output.output.wrap_err_internal("typst: compile")?;
    let pdf_opts = typst_pdf::PdfOptions {
        timestamp: world.now.today(None).map(typst_pdf::Timestamp::new_utc),
        ..Default::default()
    };
    typst_pdf::pdf(&doc, &pdf_opts).wrap_err_internal("typst: pdf")
}

#[cfg(test)]
mod tests {
    use super::{Data, Language, OurWonderfulTypstWorldBase, compile};

    fn empty_data() -> Data {
        Data {
            language: Language::Swedish,
            activity_name: "Testaktivitet".to_owned(),
            creator_name: "Testsektionen".to_owned(),
            creator_logo_format: None,
            creator_logo_data: None,
            fees: 0,
            fees_external: 0,
            per_object: Vec::new(),
            per_alcohol_category: Vec::new(),
            receipt_count: 0,
            receipts: Vec::new(),
        }
    }

    #[test]
    fn appends_pdf_receipts() -> minilith_errors::MinilithResult<()> {
        let world = OurWonderfulTypstWorldBase::default();
        let receipt = compile(&world, &empty_data())?;
        let mut report = empty_data();
        report.receipt_count = 1;
        report.receipts.push(bytes::Bytes::from(receipt));

        compile(&world, &report)?;
        Ok(())
    }
}
