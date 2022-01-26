use rsa::PublicKey;
use crate::encryption::PublicKey as PubK;
use crate::error::SshErrorKind;
use crate::packet::Data;
use crate::SshError;

fn u8s_to_u32s(v: Vec<u8>) -> Vec<u32> {
    let mut vec = vec![];
    for x in v {
        vec.push(x as u32)
    }
    vec
}

pub(crate) struct RSA;

impl PubK for RSA {
    fn new() -> Self where Self: Sized {
        Self
    }

    fn verify_signature(&self, ks: &[u8], message: &[u8], sig: &[u8]) -> Result<bool, SshError> {

        let mut data = Data((&ks[4..]).to_vec());
        data.get_u8s();

        let e = rsa::BigUint::from_bytes_be(data.get_u8s().as_slice());
        let n = rsa::BigUint::from_bytes_be(data.get_u8s().as_slice());
        let public_key = rsa::RsaPublicKey::new(n, e).unwrap();
        let scheme = rsa::PaddingScheme::PKCS1v15Sign {
            hash: Some(rsa::Hash::SHA1)
        };

        let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, message);
        let msg = digest.as_ref();

        Ok(public_key.verify(scheme, msg, sig).is_ok())
    }
}