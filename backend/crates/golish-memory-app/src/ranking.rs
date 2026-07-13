use golish_memory_domain::{ContextAuthority, ContextItem, KnowledgeValue};

pub fn stable_rank(items: &mut [ContextItem]) {
    items.sort_by(|left, right| {
        right
            .score_micros
            .cmp(&left.score_micros)
            .then_with(|| authority_rank(left.authority).cmp(&authority_rank(right.authority)))
            .then_with(|| left.item_id.cmp(&right.item_id))
            .then_with(|| left.content_hash.cmp(&right.content_hash))
    });
}

pub fn estimated_tokens(item: &ContextItem) -> usize {
    let value_chars = match &item.value {
        KnowledgeValue::Text(value) => value.chars().count(),
        KnowledgeValue::Json(value) => value.to_string().chars().count(),
        KnowledgeValue::VaultRef(_) => 48,
    };
    (value_chars + item.source_label.chars().count())
        .div_ceil(4)
        .max(1)
}

const fn authority_rank(authority: ContextAuthority) -> u8 {
    match authority {
        ContextAuthority::CanonicalDb => 0,
        ContextAuthority::Runtime => 1,
        ContextAuthority::Handoff => 2,
        ContextAuthority::Episode => 3,
        ContextAuthority::Assertion => 4,
        ContextAuthority::Document => 5,
        ContextAuthority::TemporalGraph => 6,
        ContextAuthority::Vector => 7,
    }
}
