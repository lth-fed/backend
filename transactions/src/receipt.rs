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

use crate::Provider;
use crate::api::Ware;

const TYPST_DOC: &str = include_str!("./receipt.typ");

#[derive(Serialize, Debug, Clone, Copy, Enum)]
pub enum Language {
    #[serde(rename = "sv")]
    #[oai(rename = "sv")]
    Swedish,
    #[serde(rename = "en")]
    #[oai(rename = "en")]
    English,
    // ALSO UPDATE `minilith/transactions.rs`
}

#[derive(Serialize, Debug, Clone)]
pub struct Data {
    pub language: Language,
    /// Ordernummer.
    pub transaction_id: String,
    pub purchase_date: String,
    /// Betalsätt.
    pub provider: Provider,
    /// Betalningsreferens.
    pub payment_reference: String,
    /// Returreferens.
    pub refund_reference: Option<String>,
    /// Varor.
    pub wares: Vec<Ware>,

    pub customer_name: Option<String>,
    pub customer_id: Option<String>,

    pub merchant_id: String,
    pub merchant_name: String,
    pub merchant_org_id: String,
    pub merchant_email: String,
    pub merchant_address: String,
    pub merchant_svg_icon: Option<String>,
}

pub struct OurWonderfulTypstWorldBase {
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
            "data.json" => return Err(FileError::NotSource),
            _ => return Err(FileError::NotFound(id.vpath().get_without_slash().into())),
        };
        Ok(data)
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if *id.root() != VirtualRoot::Project {
            return Err(FileError::Package(PackageError::Other(None)));
        }
        let data = match id.vpath().get_without_slash() {
            "receipt.typ" => Bytes::new(TYPST_DOC),
            "data.json" => Bytes::new(serde_json::to_string(&self.data).map_err(|error| {
                alert(AlertLevel::L2, "Failed to serialize data.json for receipt.");
                error!(?error, "Failed to serialize data.json for receipt.");
                FileError::Other(None)
            })?),
            _ => return Err(FileError::NotFound(id.vpath().get_without_slash().into())),
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
/// document).
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
    use crate::Provider;
    use crate::api::{Currency, Ware};

    use super::{Data, Language, OurWonderfulTypstWorldBase, compile};

    #[test]
    fn compiles_bookkeeping_receipt_without_customer_name() {
        let result = compile(
            &OurWonderfulTypstWorldBase::default(),
            &Data {
                language: Language::Swedish,
                transaction_id: "00000000-0000-0000-0000-000000000000".to_owned(),
                purchase_date: "2026-08-13".to_owned(),
                provider: Provider::Swish,
                payment_reference: "payment-reference".to_owned(),
                refund_reference: None,
                wares: vec![Ware {
                    name: "Biljett".to_owned(),
                    amount: 12_500,
                    tax: 1.25,
                    currency: Currency::Sek,
                }],
                customer_name: None,
                customer_id: None,
                merchant_id: "merchant".to_owned(),
                merchant_name: "Merchant".to_owned(),
                merchant_org_id: "000000-0000".to_owned(),
                merchant_email: "merchant@example.com".to_owned(),
                merchant_address: "Address".to_owned(),
                merchant_svg_icon: None,
            },
        );
        assert!(result.is_ok(), "bookkeeping receipt should compile");
        if let Ok(pdf) = result {
            assert!(pdf.starts_with(b"%PDF"), "receipt should be a PDF");
        }
    }
}
