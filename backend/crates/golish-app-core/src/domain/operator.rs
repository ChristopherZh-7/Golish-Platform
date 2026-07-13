//! Opaque, server-owned operator identity used by privileged local actions.
//!
//! The principal intentionally implements neither `Serialize` nor
//! `Deserialize`; command request DTOs must obtain it from a trusted provider.
//!
//! ```compile_fail
//! # use golish_app_core::domain::operator::TrustedOperatorPrincipal;
//! let _: TrustedOperatorPrincipal = serde_json::from_str("{}").unwrap();
//! ```
//!
//! Fields are private, so an application request adapter cannot build a fake
//! principal with a caller-selected UUID.
//!
//! ```compile_fail
//! # use golish_app_core::domain::operator::{OperatorChannel, OperatorId, TrustedOperatorPrincipal};
//! # let id = OperatorId::from_server_record(uuid::Uuid::nil());
//! let _ = TrustedOperatorPrincipal { id, channel: OperatorChannel::LocalDesktop };
//! ```

use async_trait::async_trait;
use uuid::Uuid;

use crate::GolishError;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct OperatorId(Uuid);

impl OperatorId {
    /// Construct only after a server-owned repository returned the row.
    pub fn from_server_record(id: Uuid) -> Self {
        Self(id)
    }

    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum OperatorChannel {
    LocalDesktop,
    LocalCli,
}

impl OperatorChannel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalDesktop => "local_desktop",
            Self::LocalCli => "local_cli",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedOperatorPrincipal {
    id: OperatorId,
    channel: OperatorChannel,
}

impl TrustedOperatorPrincipal {
    /// Construct from a row loaded by a trusted server-side provider. This type
    /// cannot cross serde/IPC boundaries and exposes no request DTO conversion.
    pub fn from_server_record(id: Uuid, channel: OperatorChannel) -> Self {
        Self {
            id: OperatorId::from_server_record(id),
            channel,
        }
    }

    pub const fn id(&self) -> OperatorId {
        self.id
    }

    pub const fn channel(&self) -> OperatorChannel {
        self.channel
    }
}

#[async_trait]
pub trait TrustedOperatorPrincipalProvider: Send + Sync {
    async fn current(
        &self,
        channel: OperatorChannel,
    ) -> Result<TrustedOperatorPrincipal, GolishError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_retains_server_identity_and_local_channel() {
        let id = Uuid::new_v4();
        let principal =
            TrustedOperatorPrincipal::from_server_record(id, OperatorChannel::LocalDesktop);
        assert_eq!(principal.id().as_uuid(), id);
        assert_eq!(principal.channel(), OperatorChannel::LocalDesktop);
        assert_eq!(principal.channel().as_str(), "local_desktop");
    }

    #[test]
    fn source_contract_has_no_serde_derive_or_public_fields() {
        let source = include_str!("operator.rs");
        let principal = source
            .split("pub struct TrustedOperatorPrincipal")
            .nth(1)
            .expect("principal declaration");
        let declaration = principal.split('}').next().expect("principal body");
        assert!(!declaration.contains("pub id"));
        assert!(!declaration.contains("pub channel"));
        let serialize_then_deserialize = ["Serialize", ", Deserialize"].concat();
        let deserialize_then_serialize = ["Deserialize", ", Serialize"].concat();
        assert!(!source.contains(&serialize_then_deserialize));
        assert!(!source.contains(&deserialize_then_serialize));
    }
}
