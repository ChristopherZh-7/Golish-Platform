//! Graph execution engine — **vendored** from metalcraft (`rust4ai/metalcraft`,
//! MIT, depth-1 clone read 2026-06-02), per Engine v2 design §2.2 (留-搓-借:
//! borrow metalcraft's graph executor + Checkpointer + deterministic parallel
//! merge as a controlled in-tree module, **not** an external dependency —
//! bus-factor-1 / pre-1.0 supply-chain risk, see gap analysis 附录 A.4).
//!
//! Provenance / faithfulness (gap 附录 A line-level assertions verified against
//! the cloned source this session):
//!   - `RunOutcome::Failed { state, node, error }` preserves partial state
//!   - `execute_parallel` uses `FuturesUnordered` + `sort_by` for deterministic
//!     merge ordering
//!   - `Checkpointer` trait + `MemoryCheckpointer`; HITL interrupt + `resume`
//!
//! Local adaptations:
//!   - `crate::{error,graph,executor,checkpoint}` paths → `super::*`
//!   - dropped the `Executor::stream()` method (its `tokio-stream` / `mpsc`
//!     dependency is not in this crate's tree and the harness uses run/resume,
//!     not streaming); the rest is faithful.

pub mod checkpoint;
pub mod error;
pub mod executor;
pub mod graph;

pub use checkpoint::{Checkpointer, MemoryCheckpointer};
pub use error::{GraphError, Result};
pub use executor::{
    Executor, GuardAction, RunOutcome, StepEvent, StepGuard, StepObserver, StepOutcome,
};
pub use graph::{
    CompiledGraph, CondFn, Edge, Graph, Node, NodeOutcome, Reducer, SubgraphNode, END, START,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, Default)]
    struct Counter {
        n: i32,
    }
    enum Upd {
        Inc(i32),
    }
    impl Reducer for Counter {
        type Update = Upd;
        fn apply(&mut self, u: Upd) {
            match u {
                Upd::Inc(d) => self.n += d,
            }
        }
    }

    #[tokio::test]
    async fn linear_graph_runs_to_completion() {
        let g = Graph::<Counter>::new()
            .add_node("a", |_s: Counter| async {
                Ok(NodeOutcome::Update(Upd::Inc(1)))
            })
            .add_node("b", |_s: Counter| async {
                Ok(NodeOutcome::Update(Upd::Inc(41)))
            })
            .add_edge("a", "b")
            .add_edge("b", END)
            .set_entry("a")
            .compile()
            .expect("compile");
        match Executor::new(g)
            .run(Counter::default(), "t1")
            .await
            .expect("run")
        {
            RunOutcome::Completed(s) => assert_eq!(s.n, 42),
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn memory_checkpointer_round_trips() {
        let cp = MemoryCheckpointer::<Counter>::new();
        let mut st = Counter::default();
        st.apply(Upd::Inc(7));
        cp.save("th", &st, "next").await.expect("save");
        let (loaded, next) = cp.load("th").await.expect("load").expect("some");
        assert_eq!(loaded.n, 7);
        assert_eq!(next, "next");
    }

    #[test]
    fn to_mermaid_lists_edges() {
        let g = Graph::<Counter>::new()
            .add_node("a", |_s: Counter| async {
                Ok(NodeOutcome::Update(Upd::Inc(1)))
            })
            .add_edge("a", END)
            .set_entry("a")
            .compile()
            .expect("compile");
        let m = g.to_mermaid();
        assert!(m.contains("flowchart TD"));
        assert!(m.contains(&format!("a --> {END}")));
    }
}
