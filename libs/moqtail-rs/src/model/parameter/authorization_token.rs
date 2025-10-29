use crate::model::{
  common::{varint::BufMutVarIntExt, varint::BufVarIntExt},
  error::ParseError,
  parameter::constant::TokenAliasType,
};
use bytes::{Bytes, BytesMut};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationToken {
  pub alias_type: u64,
  pub token_alias: Option<u64>,
  pub token_type: Option<u64>,
  pub token_value: Option<Bytes>,
}

impl AuthorizationToken {
  pub fn new_delete(token_alias: u64) -> Self {
    Self {
      alias_type: TokenAliasType::Delete as u64,
      token_alias: Some(token_alias),
      token_type: None,
      token_value: None,
    }
  }
  pub fn new_register(token_alias: u64, token_type: u64, token_value: Bytes) -> Self {
    Self {
      alias_type: TokenAliasType::Register as u64,
      token_alias: Some(token_alias),
      token_type: Some(token_type),
      token_value: Some(token_value),
    }
  }

  pub fn new_use_alias(token_alias: u64) -> Self {
    Self {
      alias_type: TokenAliasType::UseAlias as u64,
      token_alias: Some(token_alias),
      token_type: None,
      token_value: None,
    }
  }

  pub fn new_use_value(token_type: u64, token_value: Bytes) -> Self {
    Self {
      alias_type: TokenAliasType::UseValue as u64,
      token_alias: None,
      token_type: Some(token_type),
      token_value: Some(token_value),
    }
  }
  pub fn serialize(&self) -> Result<Bytes, ParseError> {
    let mut buf = BytesMut::new();
    buf.put_vi(self.alias_type)?;
    match self.alias_type {
      x if x == TokenAliasType::Delete as u64 => {
        if let Some(token_alias) = self.token_alias {
          buf.put_vi(token_alias)?;
        }
      }
      x if x == TokenAliasType::Register as u64 => {
        if let (Some(token_alias), Some(token_type), Some(token_value)) =
          (self.token_alias, self.token_type, self.token_value.clone())
        {
          buf.put_vi(token_alias)?;
          buf.put_vi(token_type)?;
          buf.extend_from_slice(&token_value);
        }
      }
      x if x == TokenAliasType::UseAlias as u64 => {
        if let Some(token_alias) = self.token_alias {
          buf.put_vi(token_alias)?;
        }
      }
      x if x == TokenAliasType::UseValue as u64 => {
        if let (Some(token_type), Some(token_value)) = (self.token_type, self.token_value.clone()) {
          buf.put_vi(token_type)?;
          buf.extend_from_slice(&token_value);
        }
      }
      x => {
        return Err(ParseError::InvalidType {
          context: "AuthorizationToken::serialize: unknown alias_type",
          details: format!("invalid alias_type {}", x),
        });
      }
    }

    Ok(buf.freeze())
  }

  pub fn deserialize(buf: &mut Bytes) -> Result<Self, ParseError> {
    let alias_type = buf.get_vi()?;
    let alias_type = TokenAliasType::try_from(alias_type)?;
    match alias_type {
      TokenAliasType::Delete => {
        let token_alias = buf.get_vi()?;
        Ok(AuthorizationToken::new_delete(token_alias))
      }
      TokenAliasType::Register => {
        let token_alias = buf.get_vi()?;
        let token_type = buf.get_vi()?;
        let token_value = buf.clone();
        Ok(AuthorizationToken::new_register(
          token_alias,
          token_type,
          token_value,
        ))
      }
      TokenAliasType::UseAlias => {
        let token_alias = buf.get_vi()?;
        Ok(AuthorizationToken::new_use_alias(token_alias))
      }
      TokenAliasType::UseValue => {
        let token_type = buf.get_vi()?;
        let token_value = buf.clone();
        Ok(AuthorizationToken::new_use_value(token_type, token_value))
      }
    }
  }
}
