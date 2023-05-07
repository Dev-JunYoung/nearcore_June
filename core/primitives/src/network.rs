use crate::hash::CryptoHash;
use crate::types::{AccountId, EpochId};
use borsh::{BorshDeserialize, BorshSerialize};
use near_crypto::{KeyType, PublicKey, SecretKey, Signature};
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

/// Peer id is the public key.
#[derive(
    BorshSerialize,
    BorshDeserialize,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct PeerId(Arc<PublicKey>); //struct PeerId(PublicKey);

impl PeerId {
    pub fn new(key: PublicKey) -> Self {
        Self(Arc::new(key))
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.0
    }
}

impl PeerId {
    pub fn random() -> Self {
        PeerId::new(SecretKey::from_random(KeyType::ED25519).public_key())
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Account announcement information
/// 계정 공지 정보
#[derive(BorshSerialize, BorshDeserialize, PartialEq, Eq, Clone, Debug, Hash)]
pub struct AnnounceAccount { //각 유효성 검사기는 기본적으로 이 구저체를 전체 네트워크에 정기적으로 브로드캐스트. 출처:https://near.github.io/nearcore/architecture/how/tx_routing.html
    /// AccountId to be announced.
    pub account_id: AccountId,
    /// PeerId from the owner of the account.
    pub peer_id: PeerId,
    /// This announcement is only valid for this `epoch`.
    pub epoch_id: EpochId,
    /// Signature using AccountId associated secret key.
    /// AccountId와 연결된 비밀 키를 사용하여 서명합니다.
    pub signature: Signature,
}

impl AnnounceAccount {
    /// We hash only (account_id, peer_id, epoch_id). There is no need hash the signature
    /// as it's uniquely determined the the triple.
    pub fn build_header_hash(
        account_id: &AccountId,
        peer_id: &PeerId,
        epoch_id: &EpochId,
    ) -> CryptoHash {
        CryptoHash::hash_borsh((account_id, peer_id, epoch_id))
    }

    pub fn hash(&self) -> CryptoHash {
        AnnounceAccount::build_header_hash(&self.account_id, &self.peer_id, &self.epoch_id)
    }
}
