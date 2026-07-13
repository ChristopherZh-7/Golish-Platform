/// P7a installs the canonical kernel only. Tools remain unavailable until C8.
pub const fn p7a_tool_capabilities() -> &'static [&'static str] {
    &[]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p7a_exposes_no_callable_cleanup_tool() {
        assert!(p7a_tool_capabilities().is_empty());
    }
}
