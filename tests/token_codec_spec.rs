use n2f_rs::{
    domains::identity::{
        domain::token::TokenPurpose,
        token_codec::{Codec, Issued},
    },
    shared::{
        entropy::EntropyError,
        errors::{Classified, Failure, Kind},
        secret::SecretString,
    },
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Token {
    purpose: String,
    name: String,
    secret: String,
    digest_hex: String,
}
#[derive(Deserialize)]
struct Reject {
    name: String,
    secret: String,
}
#[derive(Deserialize)]
struct Vectors {
    tokens: Vec<Token>,
    secret_length: usize,
    rejects: Vec<Reject>,
}
fn vectors() -> Vectors {
    serde_json::from_str(include_str!(
        "../src/domains/identity/token_codec/testdata/vectors.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn refusal<T>(result: Result<T, Failure>, kind: Kind, code: &str) {
    let err = match result {
        Ok(_) => panic!("expected refusal {code}"),
        Err(e) => e,
    };
    let c = err.classification().expect("classified error");
    assert_eq!(c.kind, kind, "{code}");
    assert_eq!(c.error_type, Some(code));
}
fn failing(bytes: &mut [u8]) -> Result<(), EntropyError> {
    bytes.fill(0);
    Err("entropy unavailable".into())
}
fn secret(s: &str) -> SecretString {
    SecretString::new(s.to_owned())
}
const ALL: [TokenPurpose; 4] = [
    TokenPurpose::Session,
    TokenPurpose::EmailVerification,
    TokenPurpose::PasswordReset,
    TokenPurpose::WebSocketUpgrade,
];

#[test]
fn t02_t03_t07_fixture_digests() {
    let codec = Codec::new(failing).unwrap();
    for t in vectors().tokens {
        let purpose = TokenPurpose::parse(&t.purpose).unwrap();
        let digest = codec.digest(purpose, &secret(&t.secret)).unwrap();
        assert_eq!(
            digest.bytes().to_vec(),
            hex(&t.digest_hex),
            "{}/{}",
            t.purpose,
            t.name
        );
        assert_eq!(digest.purpose(), purpose);
    }
}

#[test]
fn t02_t07_noncanonical_secrets_are_refused() {
    let codec = Codec::new(failing).unwrap();
    let v = vectors();
    assert_eq!(v.secret_length, 43);
    for r in v.rejects {
        for purpose in ALL {
            refusal(
                codec.digest(purpose, &secret(&r.secret)),
                Kind::Invalid,
                "identity.token_invalid",
            );
        }
        let _ = r.name;
    }
}

#[test]
fn t01_t04_t05_issue_round_trips_and_redacts() {
    let mut counter = 0u8;
    let codec = Codec::new(move |bytes: &mut [u8]| -> Result<(), EntropyError> {
        assert_eq!(bytes.len(), 32);
        counter = counter.wrapping_add(1);
        bytes.fill(counter);
        Ok(())
    })
    .unwrap();
    let mut seen = std::collections::HashSet::new();
    for purpose in ALL {
        let issued = codec.issue(purpose).unwrap();
        assert_eq!(issued.secret.reveal().len(), 43);
        assert!(
            issued
                .secret
                .reveal()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_eq!(issued.digest.purpose(), purpose);
        assert_eq!(
            codec.digest(purpose, &issued.secret).unwrap(),
            issued.digest
        );
        assert!(seen.insert(issued.digest.bytes()), "fresh bytes per issue");
        assert_eq!(format!("{}", issued.secret), "[REDACTED]");
        assert_eq!(format!("{:?}", issued.secret), "[REDACTED]");
        assert_eq!(format!("{:?}", issued.digest), "[REDACTED]");
    }
    // Same secret, different purpose: different digest.
    let s = secret("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8");
    let a = codec.digest(TokenPurpose::Session, &s).unwrap();
    let b = codec.digest(TokenPurpose::PasswordReset, &s).unwrap();
    assert_ne!(a.bytes(), b.bytes());
}

#[test]
fn t05_issued_has_no_debug_presentation() {
    trait NotDebug {
        const NOT_DEBUG: bool = true;
    }
    impl<T> NotDebug for T {}
    struct Check<T>(std::marker::PhantomData<T>);
    impl<T: std::fmt::Debug> Check<T> {
        #[allow(dead_code)]
        const NOT_DEBUG: bool = false;
    }
    // Compile-time: Issued lacks Debug while an ordinary type has it (the
    // inherent const shadows the trait const only when T: Debug).
    const CHECKED: bool = Check::<Issued>::NOT_DEBUG && !Check::<String>::NOT_DEBUG;
    const _: [(); 1] = [(); CHECKED as usize];
    assert!(std::hint::black_box(CHECKED));
}

#[test]
fn t01_entropy_failure_refuses_without_fallback() {
    let codec = Codec::new(failing).unwrap();
    for purpose in ALL {
        refusal(
            codec.issue(purpose),
            Kind::Unavailable,
            "identity.entropy_unavailable",
        );
    }
    let short = Codec::new(|bytes: &mut [u8]| -> Result<(), EntropyError> {
        bytes[..16].fill(9);
        Err("partial fill".into())
    })
    .unwrap();
    refusal(
        short.issue(TokenPurpose::Session),
        Kind::Unavailable,
        "identity.entropy_unavailable",
    );
}
