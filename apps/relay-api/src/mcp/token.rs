// Copyright 2026 The MOQtail Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! API tokens kept as salted hashes, so the configuration never holds the token.
//!
//! A stored value reads `sha256d$<salt>$<digest>`: a 16-byte random salt and
//! SHA-256 applied twice to the salt followed by the token, both in hex. Tokens are
//! 32 random bytes, too many to guess, so a fast hash is enough. The second pass
//! hides SHA-256's internal state, which rules out length-extension tricks on the
//! digest.

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};

const SCHEME: &str = "sha256d";
const SALT_BYTES: usize = 16;

/// A token's stored hash, parsed.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenHash {
  salt: [u8; SALT_BYTES],
  digest: [u8; 32],
}

// The digest is not secret in the way the token is, but there is no reason to
// print it in debug output either.
impl std::fmt::Debug for TokenHash {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("TokenHash(..)")
  }
}

fn digest(salt: &[u8], token: &str) -> [u8; 32] {
  let mut first = Sha256::new();
  first.update(salt);
  first.update(token.as_bytes());
  Sha256::digest(first.finalize()).into()
}

fn hex(bytes: &[u8]) -> String {
  bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
  if text.len() != N * 2 || !text.is_ascii() {
    return None;
  }
  let mut out = [0u8; N];
  for (i, byte) in out.iter_mut().enumerate() {
    *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
  }
  Some(out)
}

impl TokenHash {
  /// Hashes `token` with a fresh random salt.
  pub fn new(token: &str) -> Self {
    let salt: [u8; SALT_BYTES] = rand::random();
    TokenHash {
      salt,
      digest: digest(&salt, token),
    }
  }

  pub fn parse(text: &str) -> Result<Self> {
    let mut parts = text.trim().split('$');
    let (Some(scheme), Some(salt), Some(digest), None) =
      (parts.next(), parts.next(), parts.next(), parts.next())
    else {
      bail!("a token hash reads {SCHEME}$<salt>$<digest>; make one with `relay api-token`");
    };
    if scheme != SCHEME {
      bail!("unknown token hash scheme `{scheme}`; expected `{SCHEME}`");
    }
    match (unhex(salt), unhex(digest)) {
      (Some(salt), Some(digest)) => Ok(TokenHash { salt, digest }),
      _ => bail!(
        "the token hash's salt must be {} and its digest 64 hexadecimal digits",
        SALT_BYTES * 2
      ),
    }
  }

  /// Whether `token` is the one this hash was made from. Takes the same time
  /// whichever byte differs, so response times do not leak the digest.
  pub fn verify(&self, token: &str) -> bool {
    let given = digest(&self.salt, token);
    given
      .iter()
      .zip(self.digest)
      .fold(0u8, |acc, (a, b)| acc | (a ^ b))
      == 0
  }
}

impl std::fmt::Display for TokenHash {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{SCHEME}${}${}", hex(&self.salt), hex(&self.digest))
  }
}

/// A new token: `mcp_` and 32 random bytes in hex.
pub fn new_token() -> String {
  let bytes: [u8; 32] = rand::random();
  format!("mcp_{}", hex(&bytes))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_hash_verifies_only_its_own_token_and_round_trips() {
    let token = new_token();
    let hash = TokenHash::new(&token);
    assert!(hash.verify(&token));
    assert!(!hash.verify("mcp_wrong"));
    assert!(!hash.verify(""));
    assert_eq!(TokenHash::parse(&hash.to_string()).unwrap(), hash);
    // The same token hashes differently each time, because the salt differs.
    assert_ne!(TokenHash::new(&token).to_string(), hash.to_string());
  }

  #[test]
  fn the_digest_is_double_sha256_of_salt_and_token() {
    let salt: [u8; SALT_BYTES] = rand::random();
    let once = Sha256::digest([&salt[..], b"t"].concat());
    let twice: [u8; 32] = Sha256::digest(once).into();
    assert_eq!(digest(&salt, "t"), twice);
  }

  #[test]
  fn malformed_hashes_are_refused() {
    let zeros = format!("{}${}", "00".repeat(16), "00".repeat(32));
    assert!(TokenHash::parse(&format!("sha256d${zeros}")).is_ok());
    for bad in [
      "",
      "plain-token",
      &format!("sha256${zeros}"),
      &format!("sha256d${zeros}$extra"),
      "sha256d$zz$00",
      &format!("sha256d${}${}", "00".repeat(15), "00".repeat(32)),
    ] {
      assert!(TokenHash::parse(bad).is_err(), "{bad} is refused");
    }
  }
}
