use zeroize::Zeroize;

#[derive(Debug, PartialEq, Eq, Zeroize)]
#[zeroize(drop)]
pub struct SecretBytes(pub Vec<u8>);
