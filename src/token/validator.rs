use std::hash::Hash;
use std::marker::PhantomData;
use std::str::FromStr;

use num_traits::{FromPrimitive, ToPrimitive};

use crate::crypto::PublicKey;
use crate::error::Error;
use crate::message::SignedMessage;
use crate::policy::{PolicyCondition, PolicyCount};
use crate::token::{PolicyAccessToken, ToTokenStr};

pub struct ValidationAuthority<P, F, A, E> {
    public_key: PublicKey,
    access_token_factory: F,
    _p: PhantomData<(P, A, E)>,
}

impl<P, F, A, E> ValidationAuthority<P, F, A, E>
    where P: Hash + Eq + FromPrimitive + ToPrimitive + PolicyCount,
          F: Fn(&[u8]) -> Option<A>,
          A: PolicyAccessToken<Policy=P> {
    pub fn new(public_key: PublicKey, access_token_factory: F) -> Self {
        Self {
            public_key,
            access_token_factory,
            _p: PhantomData,
        }
    }

    fn decode_verify_check_expiration(&self, token: &str) -> Result<A, Error> {
        // 1. decode signed message
        let signed_message = SignedMessage::from_str(token)?;
        // 2. check if it is generated by trusted identity server
        if !signed_message.verify(&self.public_key) {
            return Err(Error::SignatureVerificationFail);
        }
        // 3. extract access token from payload
        let access_token: A = (self.access_token_factory)(signed_message.message())
            .ok_or(Error::BadPolicyEncoding)?;
        // 4. check if it isn't expired
        if access_token.is_expired() {
            Err(Error::ExpiredAccessToken)
        } else {
            Ok(access_token)
        }
    }

    pub fn enforce(&self, condition: PolicyCondition<P>, token: impl ToTokenStr) -> Result<A, Error> {
        let token = token.to_str().ok_or(Error::Unauthorized)?;
        let access_token = self.decode_verify_check_expiration(token)?;
        // check if policies from access token satisfy required condition
        if condition.satisfy(access_token.policies()) {
            Ok(access_token)
        } else {
            Err(Error::Forbidden)
        }
    }

    pub fn to_access_enforcer(&self, token: impl ToTokenStr) -> Result<AccessEnforcer<P, A, E>, Error> {
        let token = token.to_str().ok_or(Error::Unauthorized)?;
        self.decode_verify_check_expiration(token)
            .map(AccessEnforcer::new)
    }
}

#[derive(Clone)]
pub struct AccessEnforcer<P, A, E> {
    access_token: A,
    _p: PhantomData<(P, E)>,
}

impl<P, A, E> AccessEnforcer<P, A, E>
    where P: Hash + Eq + FromPrimitive + ToPrimitive + PolicyCount,
          A: PolicyAccessToken<Policy=P> {
    pub fn new(access_token: A) -> Self {
        Self {
            access_token,
            _p: PhantomData,
        }
    }

    pub fn into_access_token(self) -> A {
        self.access_token
    }

    pub fn enforce(&self, condition: impl Into<PolicyCondition<P>>) -> Result<&A, Error> {
        let condition = condition.into();
        if condition.satisfy(self.access_token.policies()) {
            Ok(&self.access_token)
        } else {
            Err(Error::Forbidden)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::crypto::PrivateKey;
    use crate::crypto::tests::{get_test_private_key, get_test_public_key};
    use crate::error::Error::{BadSignedMessageEncoding, ExpiredAccessToken, Forbidden, SignatureVerificationFail, Unauthorized};
    use crate::policy::PolicyCondition::*;
    use crate::policy::tests::TestPolicy;
    use crate::policy::tests::TestPolicy::{Policy1, Policy2};
    use crate::token::tests::TestAccessToken;

    use super::*;

    fn create_access_token_with_key(token: TestAccessToken, private_key: &PrivateKey) -> String {
        SignedMessage::create(token.to_bytes(), &private_key).to_string()
    }

    fn create_access_token(token: TestAccessToken) -> String {
        let private_key = PrivateKey::from_base64_encoded(&get_test_private_key()).unwrap();
        create_access_token_with_key(token, &private_key)
    }

    fn make_va() -> ValidationAuthority<TestPolicy, fn(&[u8]) -> Option<TestAccessToken>, TestAccessToken, Error> {
        ValidationAuthority::new(PublicKey::from_base64_encoded(&get_test_public_key()).unwrap(), TestAccessToken::from_bytes)
    }

    #[test]
    fn test_no_token() {
        let va = make_va();

        let x = va.enforce(NoCheck, None::<&str>);
        assert!(x.is_err());
        match x.unwrap_err() {
            Unauthorized => (),
            _ => panic!("expect {:?}", Unauthorized)
        }
    }

    #[test]
    fn test_bad_token() {
        let va = make_va();
        let x = va.enforce(NoCheck, Some("123"));
        assert!(x.is_err());
        match x.unwrap_err() {
            BadSignedMessageEncoding => (),
            _ => panic!("expect {:?}", BadSignedMessageEncoding)
        }
    }

    #[test]
    fn test_sign_by_other_keys() {
        let private_key_other = PrivateKey::from_base64_encoded("B1H3hDtRa0K0XxPC2tjD8uj2Tx3i9RlsQ7jSpl4OOIY").unwrap();
        let _public_key_other = PublicKey::from_base64_encoded("uneKfdOZUuupqMK7q1KwPFluM9zxpdIlyNntF4V1Dgs").unwrap();

        let va = make_va();

        let token = TestAccessToken::new(vec![Policy1, Policy2].into(), false);
        let access_token = create_access_token_with_key(token, &private_key_other);

        let x = va.enforce(NoCheck, Some(access_token).as_deref());
        assert!(x.is_err());
        match x.unwrap_err() {
            SignatureVerificationFail => (),
            _ => panic!("expect {:?}", SignatureVerificationFail)
        }
    }

    #[test]
    fn test_access_token() {
        let va = make_va();

        let token = create_access_token(TestAccessToken::new(vec![Policy1].into(), true));
        let x = va.enforce(NoCheck, Some(token).as_deref());
        assert!(x.is_err());
        match x.unwrap_err() {
            ExpiredAccessToken => (),
            _ => panic!("expect {:?}", ExpiredAccessToken)
        };

        let token = create_access_token(TestAccessToken::new(vec![].into(), false));
        let x = va.enforce(Contains(Policy1), Some(token).as_deref());
        assert!(x.is_err());
        match x.unwrap_err() {
            Forbidden => (),
            _ => panic!("expect {:?}", Forbidden)
        }
    }
}