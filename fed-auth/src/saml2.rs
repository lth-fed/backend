use std::collections::HashMap;
use std::io::Cursor;
use std::ops::Deref;

use base64::Engine as _;
use color_eyre::eyre::Context as _;
use minilith_errors::{MinilithErrorOptionExt as _, MinilithErrorResultExt as _, MinilithResult};
use poem::http::StatusCode;
use poem_openapi::OpenApi;
use poem_openapi::payload::{Binary, Form, Response};
use samael::metadata::{
    AttributeConsumingService, ContactPerson, ContactType, EntityDescriptor, LocalizedName,
    LocalizedUri, RequestedAttribute,
};
use samael::service_provider::{ServiceProvider, ServiceProviderBuilder};
use samael::traits::ToXml as _;
use xmltree::XMLNode;

use crate::context::{ValidatedAuthSession, ValidatedUser};
use crate::{API_DOMAIN, Context, ContextWrapper};

pub async fn get_service_provider()
-> color_eyre::Result<(ServiceProvider, openssl::pkey::PKey<openssl::pkey::Private>)> {
    // let resp = reqwest::get("https://testidpv4.lu.se/idp/shibboleth")
    let resp = reqwest::get("https://mocksaml.com/api/saml/metadata")
        .await?
        .text()
        .await?;
    let idp_metadata: EntityDescriptor = samael::metadata::de::from_str(&resp)?;

    let saml_pk = std::env::var("SAML_PRIVATE_KEY").wrap_err("`SAML_PRIVATE_KEY` not detected")?;
    let saml_pk = base64::prelude::BASE64_STANDARD
        .decode(saml_pk)
        .wrap_err("`SAML_PRIVATE_KEY` not base64 encoded")?;
    let saml_pk = openssl::pkey::PKey::from_rsa(openssl::rsa::Rsa::private_key_from_pem(&saml_pk)?)
        .wrap_err("`SAML_PRIVATE_KEY` not valid base64 encoded PEM private key")?;
    let saml_cert = std::env::var("SAML_CERTIFICATE").wrap_err("`SAML_PUBLIC_KEY` not detected")?;
    let saml_cert = base64::prelude::BASE64_STANDARD
        .decode(saml_cert)
        .wrap_err("`SAML_CERTIFICATE` not base64 encoded")?;
    let saml_cert = openssl::x509::X509::from_pem(&saml_cert)?;
    let saml_cert = saml_cert.to_der()?;
    let saml_cert = samael::crypto::CertificateDer::from(saml_cert);

    let sp = ServiceProviderBuilder::default()
        .entity_id(format!("{API_DOMAIN}/saml2/"))
        .key(saml_pk.clone())
        .certificate(saml_cert)
        .allow_idp_initiated(false)
        .force_authn(true)
        .contact_person(ContactPerson {
            contact_type: Some(ContactType::Technical.value().to_owned()),
            company: Some("E-sektionen inom TLTH".to_owned()),
            given_name: Some("Informationschef".to_owned()),
            sur_name: None,
            email_addresses: Some(vec!["informationschef@esek.se".to_owned()]),
            telephone_numbers: None,
        })
        .idp_metadata(idp_metadata)
        .acs_url(format!("{API_DOMAIN}/saml2/acs"))
        // doesn't actually exist but is required by samael to exist
        .slo_url(format!("{API_DOMAIN}/saml2/slo"))
        .build()?;
    Ok((sp, saml_pk))
}
fn add_metadata(metadata: &mut EntityDescriptor) -> MinilithResult<()> {
    let org_name = vec![
        LocalizedName {
            lang: Some("se".into()),
            value: "Utvecklarna bakom Teknologappen".into(),
        },
        LocalizedName {
            lang: Some("en".into()),
            value: "The developers behind Teknologappen".into(),
        },
    ];
    metadata.organization = Some(samael::metadata::Organization {
        organization_names: Some(org_name.clone()),
        organization_display_names: Some(org_name),
        organization_urls: Some(vec![
            LocalizedUri {
                lang: Some("se".into()),
                value: "https://teknologappen.se".into(),
            },
            LocalizedUri {
                lang: Some("en".into()),
                value: "https://teknologappen.se".into(),
            },
        ]),
    });
    let sp_desc = metadata
        .sp_sso_descriptors
        .as_mut()
        .and_then(|descs| descs.first_mut())
        .wrap_err_internal("Failed to get sp sso descriptor")?;
    metadata.contact_person = Some(vec![
        ContactPerson {
            contact_type: Some(ContactType::Technical.value().to_owned()),
            company: Some("E-sektionen inom TLTH".to_owned()),
            given_name: Some("Informationschef".to_owned()),
            sur_name: None,
            email_addresses: Some(vec!["informationschef@esek.se".to_owned()]),
            telephone_numbers: None,
        },
        ContactPerson {
            contact_type: Some(ContactType::Support.value().to_owned()),
            company: Some("E-sektionen inom TLTH".to_owned()),
            given_name: Some("Informationschef".to_owned()),
            sur_name: None,
            email_addresses: Some(vec!["informationschef@esek.se".to_owned()]),
            telephone_numbers: None,
        },
        ContactPerson {
            contact_type: Some(ContactType::Administrative.value().to_owned()),
            company: Some("E-sektionen inom TLTH".to_owned()),
            given_name: Some("Informationschef".to_owned()),
            sur_name: None,
            email_addresses: Some(vec!["informationschef@esek.se".to_owned()]),
            telephone_numbers: None,
        },
    ]);
    let attribute_name_format = "urn:oasis:names:tc:SAML:2.0:attrname-format:uri";
    sp_desc.attribute_consuming_services = Some(vec![AttributeConsumingService {
        index: 1,
        is_default: None,
        service_names: vec![
            LocalizedName {
                lang: Some("sv".into()),
                value: "Teknologappens inloggningstjänst".into(),
            },
            LocalizedName {
                lang: Some("en".into()),
                value: "The login service for teknologappen".into(),
            },
        ],
        service_descriptions: None,
        request_attributes: vec![
            RequestedAttribute {
                friendly_name: Some("samlSubjectID".into()),
                name: "urn:oasis:names:tc:SAML:attribute:subject-id".into(),
                name_format: Some(attribute_name_format.into()),
                values: None,
                is_required: Some(true),
            },
            RequestedAttribute {
                friendly_name: Some("mail".into()),
                name: "urn:oid:0.9.2342.19200300.100.1.3".into(),
                name_format: Some(attribute_name_format.into()),
                values: None,
                is_required: Some(true),
            },
            RequestedAttribute {
                friendly_name: Some("displayName".into()),
                name: "urn:oid:2.16.840.1.113730.3.1.241".into(),
                name_format: Some(attribute_name_format.into()),
                values: None,
                is_required: Some(true),
            },
        ],
    }]);
    // sp_desc.name_id_formats = Some(vec!["urn:oasis:names:tc:SAML:2.0:nameid-format:transient".into()]);
    Ok(())
}
fn add_metadata_extensions(meta: &mut xmltree::Element) -> MinilithResult<usize> {
    // the xmlns are needed for parsing, they are removed later. Copied from an example SP
    // metadata: https://metadata.qa.swamid.se/?rawXML=1361
    let security_contact_person = r#"<md:ContactPerson contactType="other" remd:contactType="http://refeds.org/metadata/contactType/security" xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:remd="http://refeds.org/metadata">
    <md:Company>E-sektionen inom TLTH</md:Company>
    <md:GivenName>Informationschef</md:GivenName>
    <md:EmailAddress>informationschef@esek.se</md:EmailAddress>
</md:ContactPerson>"#;
    let descriptor_extensions = r#"<md:Extensions xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:mdattr="urn:oasis:names:tc:SAML:metadata:attribute" xmlns:samla="urn:oasis:names:tc:SAML:2.0:assertion" xmlns:mdrpi="urn:oasis:names:tc:SAML:metadata:rpi" xmlns:mdui="urn:oasis:names:tc:SAML:metadata:ui" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" xmlns:remd="http://refeds.org/metadata">
    <mdattr:EntityAttributes>
        <samla:Attribute Name="http://macedir.org/entity-category" NameFormat="urn:oasis:names:tc:SAML:2.0:attrname-format:uri">
            <samla:AttributeValue>https://refeds.org/category/personalized</samla:AttributeValue>
        </samla:Attribute>
    </mdattr:EntityAttributes>
        <mdrpi:RegistrationInfo registrationAuthority="http://www.swamid.se/" registrationInstant="2026-05-13T11:22:11Z">
        <mdrpi:RegistrationPolicy xml:lang="en">http://swamid.se/policy/mdrps</mdrpi:RegistrationPolicy>
    </mdrpi:RegistrationInfo>
</md:Extensions>"#;
    let spsso_extensions = r#"<md:Extensions xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:mdattr="urn:oasis:names:tc:SAML:metadata:attribute" xmlns:samla="urn:oasis:names:tc:SAML:2.0:assertion" xmlns:mdrpi="urn:oasis:names:tc:SAML:metadata:rpi" xmlns:mdui="urn:oasis:names:tc:SAML:metadata:ui" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" xmlns:remd="http://refeds.org/metadata">
    <mdui:UIInfo>
        <mdui:DisplayName xml:lang="en">Teknologappen and guild logins for members of TLTH</mdui:DisplayName>
        <mdui:DisplayName xml:lang="sv">Teknologappen och sektionsinlogg för medlemmar i TLTH</mdui:DisplayName>
        <mdui:Description xml:lang="sv">Teknologappenlogin, utvecklat av E, D, och F-sektionen</mdui:Description>
        <mdui:Description xml:lang="en">Teknologappen login, developed by the E, D, and F guilds</mdui:Description>
        <mdui:InformationURL xml:lang="sv">https://teknologappen.se</mdui:InformationURL>
        <mdui:InformationURL xml:lang="en">https://teknologappen.se</mdui:InformationURL>
        <mdui:PrivacyStatementURL xml:lang="en">https://auth.teknologappen.se/privacy-statement/</mdui:PrivacyStatementURL>
        <mdui:PrivacyStatementURL xml:lang="sv">https://auth.teknologappen.se/privacy-statement/</mdui:PrivacyStatementURL>
    </mdui:UIInfo>
</md:Extensions>"#;
    let mut sec_meta = xmltree::Element::parse(Cursor::new(security_contact_person))
        .wrap_err_internal("Failed to parse metadata: {err}")?;
    let mut desc_meta = xmltree::Element::parse(Cursor::new(descriptor_extensions))
        .wrap_err_internal("Failed to parse metadata: {err}")?;
    let mut spsso_meta = xmltree::Element::parse(Cursor::new(spsso_extensions))
        .wrap_err_internal("Failed to parse metadata: {err}")?;
    meta.namespaces = desc_meta.namespaces.take();
    sec_meta.namespaces = None;
    spsso_meta.namespaces = None;

    meta.children.push(XMLNode::Element(sec_meta));
    meta.children.push(XMLNode::Element(desc_meta));

    let spsso = meta
        .get_mut_child("SPSSODescriptor")
        .wrap_err_internal("Metadata is not an object!")?;
    spsso.children.push(XMLNode::Element(spsso_meta));

    Ok(descriptor_extensions.len() + spsso_extensions.len())
}

#[derive(Clone)]
pub(crate) struct SamlRouter {
    pub context: ContextWrapper,
}
impl Deref for SamlRouter {
    type Target = Context;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
#[OpenApi]
impl SamlRouter {
    /// Returns the SAML2 metadata.
    ///
    /// The body actually is `application/xml` but since [`poem-openapi`] is cringe I can't just
    /// add a string as an XML response.
    #[oai(path = "/metadata", method = "get")]
    async fn metadata(&self) -> MinilithResult<Response<Binary<Vec<u8>>>> {
        let mut metadata = self
            .service_provider
            .metadata()
            .wrap_err_internal("Failed to get metadata")?;
        add_metadata(&mut metadata)?;

        let metadata = metadata
            .to_string()
            .wrap_err_internal("Failed to convert metadata to string")?;

        let mut meta = xmltree::Element::parse(Cursor::new(&metadata))
            .wrap_err_internal("Failed to parse metadata: {err}")?;

        let exts_len = add_metadata_extensions(&mut meta)?;

        let mut metadata = Cursor::new(Vec::with_capacity(metadata.len() + exts_len + 100));
        meta.write(&mut metadata)
            .wrap_err_internal("Failed to serialize updated metadata: {err}")?;
        let metadata = metadata.into_inner();
        Ok(
            Response::new(Binary(metadata))
                .header("content-type", "application/xml; charset=utf-8"),
        )
    }
    /// Get JWT access token and a new refresh token.
    #[oai(path = "/acs", method = "post")]
    #[allow(clippy::panic, reason = "yes")]
    async fn acs(&self, body: Form<HashMap<String, String>>) -> MinilithResult<Response<()>> {
        let ids = sqlx::query_scalar!("select id from saml2_request_id_cache")
            .fetch_all(&self.db)
            .await?;
        let ids: Vec<_> = ids.iter().map(String::as_str).collect();

        let saml_response = body
            .get("SAMLResponse")
            .wrap_err_internal("saml2: no SAMLResponse in body")?;
        let ass = self
            .service_provider
            .parse_base64_response(saml_response, Some(&ids))
            .wrap_err_internal("saml2: Invalid ACS response")?;
        let request_id = ass
            .subject
            .as_ref()
            .and_then(|sub| sub.subject_confirmations.as_ref())
            .and_then(|confs| confs.first())
            .and_then(|conf| conf.subject_confirmation_data.as_ref())
            .and_then(|conf_data| conf_data.in_response_to.as_ref())
            .wrap_err_internal("saml2: no request_id??")?;

        sqlx::query!(
            "delete from saml2_request_id_cache where id = $1",
            request_id
        )
        .execute(&self.db)
        .await?;

        let session = self
            .get_session(request_id)
            .await?
            .wrap_err_internal("saml2 response to non-existing session")?;

        println!("{ass:#?}");
        let sub = ass
            .subject
            .as_ref()
            .and_then(|sub| sub.name_id.as_ref())
            .wrap_err_internal("saml2: no sub")?;

        let user = ValidatedUser {
            sub: format!("lund-university:{}", sub.value.clone()),
            email: None,
            full_name: None,
            lth_guild: None,
        };
        self.validate_session(request_id, &user).await?;
        Ok(Response::new(()).status(StatusCode::SEE_OTHER).header(
            "location",
            self.provider_callback_next_url(request_id, &ValidatedAuthSession { session, user })
                .await
                .wrap_err_internal("noalert failed to send callback")?,
        ))
    }
}
