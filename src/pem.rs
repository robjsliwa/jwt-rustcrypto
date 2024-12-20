use crate::Error;
use rsa::pkcs1::DecodeRsaPublicKey;
use rsa::pkcs8::DecodePublicKey;
use rsa::pkcs8::EncodePublicKey;
use rsa::BigUint;
use rsa::RsaPublicKey;
use simple_asn1::ASN1Block;

use lazy_static::lazy_static;

lazy_static! {
    static ref EC_PUBLIC_KEY_OID: simple_asn1::OID = simple_asn1::oid!(1, 2, 840, 10_045, 2, 1);
    static ref RSA_PUBLIC_KEY_OID: simple_asn1::OID =
        simple_asn1::oid!(1, 2, 840, 113_549, 1, 1, 1);
    static ref ED25519_OID: simple_asn1::OID = simple_asn1::oid!(1, 3, 101, 112);
}

#[derive(Debug, PartialEq, Clone)]
pub enum PemType {
    EcPublic,
    EcPrivate,
    RsaPublic,
    RsaPrivate,
    EdPublic,
    EdPrivate,
}

/// PEM key standards
#[derive(Debug, PartialEq, Clone)]
pub enum Standard {
    Pkcs1,
    Pkcs8,
}

/// Key classification
#[derive(Debug, PartialEq)]
pub(crate) enum Classification {
    Ec,
    Ed,
    Rsa,
}

#[derive(Debug, Clone)]
pub struct PemEncodedKey {
    pub content: pem::Pem,
    pub asn1: Vec<ASN1Block>,
    pub pem_type: PemType,
    pub standard: Standard,
}

impl PemEncodedKey {
    pub fn new(input: &[u8]) -> Result<Self, Error> {
        pem::parse(input)
            .map_err(Error::from)
            .and_then(Self::process_parsed_pem)
    }

    fn process_parsed_pem(content: pem::Pem) -> Result<Self, Error> {
        // Parse the ASN.1 structure from the PEM contents
        simple_asn1::from_der(content.contents())
            .map_err(Error::from)
            .and_then(|asn1_content| match content.tag() {
                "RSA PRIVATE KEY" => Ok(Self::create_pem_key(
                    content,
                    asn1_content,
                    PemType::RsaPrivate,
                    Standard::Pkcs1,
                )),
                "RSA PUBLIC KEY" => Ok(Self::create_pem_key(
                    content,
                    asn1_content,
                    PemType::RsaPublic,
                    Standard::Pkcs1,
                )),
                "EC PRIVATE KEY" => Ok(Self::create_pem_key(
                    content,
                    asn1_content,
                    PemType::EcPrivate,
                    Standard::Pkcs1,
                )),
                "PUBLIC KEY" if Self::is_ec_public_key(&asn1_content) => Ok(Self::create_pem_key(
                    content,
                    asn1_content,
                    PemType::EcPublic,
                    Standard::Pkcs8,
                )),
                // Handle generic private, public key, or certificate tags
                tag @ ("PRIVATE KEY" | "PUBLIC KEY" | "CERTIFICATE") => {
                    // Classify the key based on its ASN.1 structure
                    let classification =
                        Self::classify_pem(&asn1_content).ok_or(Error::InvalidKeyFormat)?;

                    // Determine if the key is a private key or not
                    let is_private = tag == "PRIVATE KEY";
                    let pem_type = Self::determine_pem_type(classification, is_private);

                    // Determine the standard based on ASN.1 structure if possible
                    #[allow(clippy::if_same_then_else)]
                    let standard = if tag == "PRIVATE KEY" || tag == "PUBLIC KEY" {
                        Standard::Pkcs8 // Private and public keys are generally PKCS8 formatted if tag is generic
                    } else {
                        // TODO: sCertificates might follow a different format
                        Standard::Pkcs8 // Assuming PKCS8 as a fallback
                    };

                    Ok(Self::create_pem_key(
                        content,
                        asn1_content,
                        pem_type,
                        standard,
                    ))
                }
                _ => Err(Error::InvalidKeyFormat),
            })
    }

    fn create_pem_key(
        content: pem::Pem,
        asn1: Vec<ASN1Block>,
        pem_type: PemType,
        standard: Standard,
    ) -> Self {
        PemEncodedKey {
            content,
            asn1,
            pem_type,
            standard,
        }
    }

    fn determine_pem_type(classification: Classification, is_private: bool) -> PemType {
        match (classification, is_private) {
            (Classification::Ec, true) => PemType::EcPrivate,
            (Classification::Ec, false) => PemType::EcPublic,
            (Classification::Ed, true) => PemType::EdPrivate,
            (Classification::Ed, false) => PemType::EdPublic,
            (Classification::Rsa, true) => PemType::RsaPrivate,
            (Classification::Rsa, false) => PemType::RsaPublic,
        }
    }

    fn classify_pem(asn1: &[ASN1Block]) -> Option<Classification> {
        asn1.iter().find_map(|entry| match entry {
            // Check for the EC public key OID in the ASN.1 structure
            ASN1Block::Sequence(_, entries) => {
                if entries.iter().any(|e| matches!(e, ASN1Block::ObjectIdentifier(_, oid) if *oid == *EC_PUBLIC_KEY_OID)) {
                    Some(Classification::Ec)
                } else if entries.iter().any(|e| matches!(e, ASN1Block::ObjectIdentifier(_, oid) if *oid == *RSA_PUBLIC_KEY_OID)) {
                    Some(Classification::Rsa)
                } else if entries.iter().any(|e| matches!(e, ASN1Block::ObjectIdentifier(_, oid) if *oid == *ED25519_OID)) {
                    Some(Classification::Ed)
                } else {
                    // Recursively check nested sequences
                    Self::classify_pem(entries)
                }
            }

            // Direct OID checks for EC, RSA, and ED public key identifiers
            ASN1Block::ObjectIdentifier(_, oid) if *oid == *EC_PUBLIC_KEY_OID => Some(Classification::Ec),
            ASN1Block::ObjectIdentifier(_, oid) if *oid == *RSA_PUBLIC_KEY_OID => Some(Classification::Rsa),
            ASN1Block::ObjectIdentifier(_, oid) if *oid == *ED25519_OID => Some(Classification::Ed),

            _ => None,
        })
    }

    pub fn from_rsa_components(n: &[u8], e: &[u8]) -> Result<Self, Error> {
        let public_key = RsaPublicKey::new(BigUint::from_bytes_be(n), BigUint::from_bytes_be(e))?;
        let pub_pem = public_key.to_public_key_pem(rsa::pkcs8::LineEnding::LF)?;
        let pem = pem::parse(pub_pem)?;

        Self::process_parsed_pem(pem)
    }

    pub fn as_ec_private_key(&self) -> Result<&[u8], Error> {
        self.check_key_type(Standard::Pkcs8, PemType::EcPrivate)
            .map(|_| self.content.contents())
    }

    pub fn as_ec_public_key(&self) -> Result<&[u8], Error> {
        self.check_key_type(Standard::Pkcs8, PemType::EcPublic)
            .and_then(|_| Self::extract_first_bitstring(&self.asn1))
    }

    pub fn as_ed_private_key(&self) -> Result<&[u8], Error> {
        self.check_key_type(Standard::Pkcs8, PemType::EdPrivate)
            .map(|_| self.content.contents())
    }

    pub fn as_ed_public_key(&self) -> Result<&[u8], Error> {
        self.check_key_type(Standard::Pkcs8, PemType::EdPublic)
            .and_then(|_| Self::extract_first_bitstring(&self.asn1))
    }

    pub fn as_rsa_public_key(&self) -> Result<RsaPublicKey, Error> {
        let key = match self.standard {
            Standard::Pkcs1 => RsaPublicKey::from_pkcs1_pem(&pem::encode(&self.content))?,
            Standard::Pkcs8 => RsaPublicKey::from_public_key_pem(&pem::encode(&self.content))?,
        };

        Ok(key)
    }

    fn check_key_type(
        &self,
        expected_standard: Standard,
        expected_type: PemType,
    ) -> Result<(), Error> {
        if self.standard == expected_standard && self.pem_type == expected_type {
            Ok(())
        } else {
            Err(Error::InvalidKeyFormat)
        }
    }

    fn extract_first_bitstring(asn1: &[ASN1Block]) -> Result<&[u8], Error> {
        asn1.iter()
            .find_map(|entry| match entry {
                ASN1Block::BitString(_, _, value) => Some(value.as_ref()),
                ASN1Block::OctetString(_, value) => Some(value.as_ref()),
                ASN1Block::Sequence(_, entries) => Self::extract_first_bitstring(entries).ok(),
                _ => None,
            })
            .ok_or_else(|| Error::InvalidEcdsaKey)
    }

    fn is_ec_public_key(asn1: &[ASN1Block]) -> bool {
        asn1.iter().any(|entry| match entry {
            ASN1Block::Sequence(_, sub_entries) => {
                sub_entries.iter().any(|sub_entry| matches!(sub_entry, ASN1Block::ObjectIdentifier(_, oid) if oid == *EC_PUBLIC_KEY_OID))
            }
            _ => false,
        })
    }
}
