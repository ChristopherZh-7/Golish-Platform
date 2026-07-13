use golish_memory_domain::{KnowledgeClassification, KnowledgeValue, VaultCredentialRef};
use uuid::Uuid;

#[test]
fn classification_is_closed_to_four_values_and_vault_ref_is_a_value_kind() {
    let classes = [
        KnowledgeClassification::Public,
        KnowledgeClassification::Internal,
        KnowledgeClassification::CustomerConfidential,
        KnowledgeClassification::Restricted,
    ];
    assert_eq!(
        classes.map(KnowledgeClassification::as_str),
        ["public", "internal", "customer_confidential", "restricted",]
    );
    assert!(serde_json::from_str::<KnowledgeClassification>("\"secret_reference\"").is_err());

    let reference = Uuid::from_u128(0x21);
    let value = KnowledgeValue::VaultRef(VaultCredentialRef(reference));
    let encoded = serde_json::to_value(value).expect("vault ref value serializes");
    assert_eq!(encoded["value_kind"], "vault_ref");
    assert_eq!(encoded["value"], reference.to_string());
}
