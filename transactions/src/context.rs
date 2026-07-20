use std::path::PathBuf;

use base64::Engine as _;
use bin_common::{PgPool, setup_db};
use color_eyre::Section as _;
use color_eyre::eyre::WrapErr as _;
use ed25519_dalek::pkcs8::DecodePrivateKey as _;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::jwk::JwkSet;
use minilith_errors::MinilithResult;
use sqlx::migrate;
use tracing::error;
use uuid::Uuid;

use crate::receipt::OurWonderfulTypstWorldBase;
use crate::{Provider, swish};

pub(crate) struct CancelTransactionData {
    pub id: Uuid,
    pub callback_url_v1: String,
    pub provider: Provider,
}

#[derive(Debug)]
pub struct Context {
    pub db: PgPool,

    // swish
    pub swish_client: reqwest::Client,

    // our api to those using us
    pub client: reqwest::Client,
    pub jwks: JwkSet,
    pub encoding_key: EncodingKey,

    // refunds typst
    pub typst_world: OurWonderfulTypstWorldBase,
}
impl Context {
    fn get_jwt_keys() -> color_eyre::Result<(EncodingKey, JwkSet)> {
        let key = std::env::var("PRIVATE_KEY").wrap_err("`PRIVATE_KEY` not detected")?;
        let key = base64::prelude::BASE64_STANDARD
            .decode(key)
            .wrap_err("`PRIVATE_KEY` not base64 encoded")?;
        let _signing_key = ed25519_dalek::SigningKey::from_pkcs8_der(&key)?;
        let encoding_key = EncodingKey::from_ed_der(&key);

        // let jwk = fed_auth_verifier::eddsa_to_jwk(&signing_key.verifying_key());
        let keys = JwkSet {
            keys: vec![/*jwk*/],
        };
        Ok((encoding_key, keys))
    }
    /// # Errors
    ///
    /// Returns any errors stemming from setting up the DB or other services.
    pub async fn new(test_db: Option<PgPool>) -> color_eyre::Result<Self> {
        let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

        let (encoding_key, jwks) = Self::get_jwt_keys()?;

        let db = if let Some(db) = test_db {
            db
        } else {
            setup_db(
                &std::env::var("DATABASE_URL").wrap_err("`DATABASE_URL` not set")?,
                Some(migrate!("./migrations")),
            )
            .await
            .wrap_err("Failed to set up the database")
            .suggestion("Start the database with `docker compose up -d`")?
        };

        let cert = std::env::var("SWISH_CERT").wrap_err("No `SWISH_CERT` env variable")?;
        let key = std::env::var("SWISH_KEY").wrap_err("No `SWISH_KEY` env variable")?;
        let rustls_buf = format!("{key}\n{cert}");

        let swish_client = reqwest::Client::builder()
            .identity(
                reqwest::Identity::from_pem(rustls_buf.as_bytes())
                    .wrap_err("failed to build client authentication from env certs")?,
            )
            .build()
            .wrap_err("Failed to build swish client")?;

        //         let resp = swish_client.put("https://mss.cpc.getswish.net/swish-cpcapi/api/v2/paymentrequests/11A86BE70EA346E4B1C39C874173F088").header("content-type", "application/json").body(r#"{
        //     "payeePaymentReference": "0123456789",
        //     "callbackUrl": "https://example.com/api/swishcb/paymentrequests",
        //     "payerAlias": "4671234768",
        //     "payeeAlias": "1234679304",
        //     "amount": "100",
        //     "currency": "SEK",
        //     "message": "Kingston USB Flash Drive 8 GB",
        //     "callbackIdentifier": "11A86BE70EA346E4B1C39C874173F478"
        // }"#).send().await?;
        //
        //         println!("{resp:?}");

        let typst_world = OurWonderfulTypstWorldBase::default();

        let data = crate::receipt::Data {
            transaction_id: "hi".to_owned(),
            purchase_date: "today".to_owned(),
            provider: Provider::Swish,
            payment_reference: "1234".to_owned(),
            refund_refrence: None,
            wares: vec![crate::api::Ware {
                name: "Sittningsbiljett".to_owned(),
                amount: 10000,
                tax: 1.25,
                currency: crate::api::Currency::Sek,
            }],
            customer_name: "Erik Davidsson".to_owned(),
            merchant_id: "esek".to_owned(),
            merchant_name: "E-sektionen inom TLTH".to_owned(),
            merchant_org_id: "845001-2284".to_owned(),
            merchant_email: "informationschef@esek.se".to_owned(),
            merchant_address: "Edekvata, plan B i E-huset, LTH \n\
                Ole Römers väg 3B \n\
                223 63 Lund"
                .to_owned(),
        };
        let pdf = crate::receipt::compile(&typst_world, &data);
        tokio::fs::write("./hej.pdf", &pdf).await?;

        let context = Self {
            db,
            swish_client,
            client: reqwest::Client::new(),
            jwks,
            encoding_key,

            typst_world,
        };
        Ok(context)
    }

    /// # Return
    ///
    /// Returns `true` if cancel is successful.
    pub(crate) async fn cancel_transaction(
        &self,
        transaction: &CancelTransactionData,
    ) -> MinilithResult<bool> {
        match transaction.provider {
            Provider::Swish => {
                let patch = vec![swish::PaymentRequestPatch {
                    op: swish::PaymentRequestPatchOperation::Replace,
                    path: "/status".to_owned(),
                    value: "cancelled".to_owned(),
                }];
                let resp = match self
                    .swish_client
                    .patch(swish::payment_request_url(transaction.id))
                    .json(&patch)
                    .send()
                    .await
                {
                    Ok(resp) => resp,
                    Err(err) => {
                        // ALERT LEVEL 2
                        error!(
                            ?err,
                            "failed to cancel swish payment request due to connection issues"
                        );
                        return Ok(false);
                    }
                };
                if resp.status().is_success() {
                    return Ok(true);
                }

                let text = match resp.text().await {
                    Ok(text) => text,
                    Err(err) => {
                        error!(?err, "failed to read body of cancel swish payment request");
                        // ALERT LEVEL 2
                        return Ok(false);
                    }
                };
                // non-cancellable state
                if text.contains("RP07") {
                    return Ok(false);
                }

                error!("Shit went down with swish cancel!");
                // ALERT LEVEL 1
            }
            Provider::Stripe => todo!(),
        }
        Ok(false)
    }
}
