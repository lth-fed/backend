use std::fmt::Debug;

use minilith_errors::{MinilithErrorResultExt as _, MinilithResult};
use poem_openapi::Enum;
use serde::Serialize;
use tracing::{info_span, warn};
use typst::diag::{FileError, FileResult, PackageError};
use typst::foundations::Bytes;
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt as _};
use typst_layout::PagedDocument;

const TYPST_DOC: &str = include_str!("./report.typ");
/// Shared helpers imported by every template (e.g. currency formatting).
const TYPST_FORMAT: &str = include_str!("./format.typ");

#[derive(Serialize, Debug, Clone, Copy, Enum)]
pub enum Language {
    #[serde(rename = "sv")]
    #[oai(rename = "sv")]
    Swedish,
    #[serde(rename = "en")]
    #[oai(rename = "en")]
    English,
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
    pub is_group: bool,
    pub creator_name: String,
    pub creator_logo_format: Option<ImageKind>,
    #[serde(skip)]
    #[allow(clippy::struct_field_names, reason = "stfu")]
    pub creator_logo_data: Option<bytes::Bytes>,
    pub fees: i64,
    pub fees_external: i64,
    /// Fees charged by the provider for refunding payments (kept apart from `fees`).
    pub fees_refund: i64,
    pub per_object: Vec<Object>,
    /// Purchases refunded by an admin, in the same grouping as `per_object` but with the
    /// refunded amount as the price. Resales are not refunds and never show up here.
    pub per_refund: Vec<Object>,
    pub per_alcohol_category: Vec<AlcoholCategory>,
    pub exported_at: Option<String>,
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
/// A typst document plus the JSON it reads from `data.json` (and optionally `image`).
pub struct Template {
    pub source: &'static str,
    pub data_json: String,
    /// Bytes served as the `image` file, e.g. a creator logo.
    pub image: Option<bytes::Bytes>,
}

struct OurWonderfulTypstWorld<'a> {
    inner: &'a OurWonderfulTypstWorldBase,
    now: typst_kit::datetime::Time,

    template: &'a Template,
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
            VirtualPath::new("report.typ").unwrap(),
        ))
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if *id.root() != VirtualRoot::Project {
            return Err(FileError::Package(PackageError::Other(None)));
        }
        let data = match id.vpath().get_without_slash() {
            "report.typ" => Source::new(id, self.template.source.to_owned()),
            "format.typ" => Source::new(id, TYPST_FORMAT.to_owned()),
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
            "report.typ" => Bytes::new(self.template.source),
            "format.typ" => Bytes::new(TYPST_FORMAT),
            "data.json" => Bytes::new(self.template.data_json.clone()),
            "image" if let Some(data) = &self.template.image => {
                Bytes::new(bytes::Bytes::clone(data))
            }
            _ => return Err(FileError::NotFound(path.into())),
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
    let template = Template {
        source: TYPST_DOC,
        data_json: serde_json::to_string(data).wrap_err_internal("report: serialize data.json")?,
        image: data.creator_logo_data.clone(),
    };
    compile_template(world, &template)
}

/// Compiles any typst template with its data. Used by every PDF export so the world, fonts and
/// PDF options are shared.
pub fn compile_template(
    world: &OurWonderfulTypstWorldBase,
    template: &Template,
) -> MinilithResult<Vec<u8>> {
    let _span = info_span!("typst report compilation").entered();

    let world = OurWonderfulTypstWorld {
        inner: world,
        now: typst_kit::datetime::Time::system(),
        template,
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
            is_group: false,
            creator_name: "Testsektionen".to_owned(),
            creator_logo_format: None,
            creator_logo_data: None,
            fees: 0,
            fees_external: 0,
            fees_refund: 0,
            per_object: Vec::new(),
            per_refund: Vec::new(),
            per_alcohol_category: Vec::new(),
            exported_at: None,
        }
    }

    #[test]
    fn compiles_sales_report() -> minilith_errors::MinilithResult<()> {
        let world = OurWonderfulTypstWorldBase::default();
        for language in [Language::Swedish, Language::English] {
            let mut report = empty_data();
            report.language = language;
            // Exercises the refunds section and the conditional summary rows.
            report.per_refund.push(super::Object {
                name: "Testbiljett".to_owned(),
                price: 10_000,
                number: 1,
            });
            report.fees_refund = 100;
            let pdf = compile(&world, &report)?;
            assert!(pdf.starts_with(b"%PDF"));
        }
        Ok(())
    }
}
