#![allow(
    clippy::unwrap_used,
    reason = "we control all the inputs except the json inputs, therefore unwraps in typst is safe"
)]

use std::fmt::Debug;

use serde::Serialize;
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

#[derive(Serialize, Debug, Clone)]
pub struct Data {
    /// Ordernummer.
    pub transaction_id: String,
    pub purchase_date: String,
    /// Betalsätt.
    pub provider: Provider,
    /// Betalningsreferens.
    pub payment_reference: String,
    /// Returreferens.
    pub refund_refrence: Option<String>,
    /// Varor.
    pub wares: Vec<Ware>,

    pub customer_name: String,

    pub merchant_id: String,
    pub merchant_name: String,
    pub merchant_org_id: String,
    pub merchant_email: String,
    pub merchant_address: String,
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

    fn main(&self) -> FileId {
        FileId::new(RootedPath::new(
            VirtualRoot::Project,
            VirtualPath::new("main.typ").unwrap(),
        ))
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if *id.root() != VirtualRoot::Project {
            return Err(FileError::Package(PackageError::Other(None)));
        }
        let data = match id.vpath().get_without_slash() {
            "main.typ" => Source::new(id, TYPST_DOC.to_owned()),
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
            "main.typ" => Bytes::new(TYPST_DOC),
            "data.json" => Bytes::new(serde_json::to_string(&self.data).unwrap()),
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
/// # Panics
///
/// Panics if the typst compilation fails (which should never happen; we control the typst
/// document).
pub fn compile(world: &OurWonderfulTypstWorldBase, data: &Data) -> Vec<u8> {
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
    let doc = output.output.unwrap();
    let pdf_opts = typst_pdf::PdfOptions {
        timestamp: Some(typst_pdf::Timestamp::new_utc(
            world.now.today(None).unwrap(),
        )),
        ..Default::default()
    };
    typst_pdf::pdf(&doc, &pdf_opts).unwrap()
}
