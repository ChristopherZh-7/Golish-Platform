import { useStore } from "./index";

export function installDevTools() {
  if (!import.meta.env.DEV) return;

  (window as unknown as { __GOLISH_STORE__: typeof useStore }).__GOLISH_STORE__ = useStore;

  (window as unknown as Record<string, unknown>).__mockPlan = (variant?: "active" | "done" | "retired") => {
    const state = useStore.getState();
    const sid = state.activeSessionId;
    if (!sid) { console.warn("No active session"); return; }

    const v = variant ?? "active";
    const now = Date.now();

    let convId = state.activeConversationId;
    if (!convId) {
      convId = `mock-conv-${now}`;
      state.addConversation({
        id: convId,
        title: "Security Assessment",
        messages: [],
        createdAt: now,
        aiSessionId: sid,
        aiInitialized: true,
        isStreaming: false,
      });
    }

    const planMsgId = `mock-plan-msg-${now}`;
    state.addConversationMessage(convId, {
      id: `mock-user-${now}`,
      role: "user",
      content: "Scan http://8.138.179.62:8080/ for vulnerabilities",
      timestamp: now - 60000,
    });

    state.addConversationMessage(convId, {
      id: `mock-asst-${now}`,
      role: "assistant",
      content: "I'll analyze the target and create a comprehensive security assessment plan.",
      timestamp: now - 55000,
    });

    state.addConversationMessage(convId, {
      id: planMsgId,
      role: "assistant",
      content: "I've created the following plan and will begin execution now.",
      timestamp: now - 50000,
      toolCalls: [{ name: "update_plan", args: "{}", requestId: "tc-plan" }],
    });

    useStore.setState((s) => {
      if (s.sessions[sid]) {
        s.sessions[sid].planMessageId = planMsgId;
      }
    });

    const plan = {
      version: 1,
      summary: {
        total: 5,
        completed: v === "done" ? 5 : v === "active" ? 2 : 3,
        in_progress: v === "active" ? 1 : 0,
        failed: 0,
      },
      steps: [
        { id: "s1", step: "Verify target registration in database", status: (v === "done" || v === "active" ? "completed" : "completed") as string },
        { id: "s2", step: "Execute reconnaissance pipeline against target", status: (v === "done" || v === "active" ? "completed" : "completed") as string },
        { id: "s3", step: "Run port scanning and service fingerprinting", status: (v === "done" ? "completed" : v === "active" ? "in_progress" : "completed") as string },
        { id: "s4", step: "Perform vulnerability assessment with nuclei", status: (v === "done" ? "completed" : "pending") as string },
        { id: "s5", step: "Generate final security report", status: (v === "done" ? "completed" : "pending") as string },
      ],
    };
    state.setPlan(sid, plan as Parameters<typeof state.setPlan>[1]);

    const toolExecs = [
      { id: "te-1", tool: "run_pty_cmd", step: "s1", args: { command: "sqlite3 targets.db 'SELECT * FROM targets'" }, status: "completed" as const, result: '{"stdout": "id|url|status\\n1|http://8.138.179.62:8080/|active"}' },
      { id: "te-2", tool: "run_pty_cmd", step: "s2", args: { command: "nmap -sV -p 1-1000 8.138.179.62" }, status: "completed" as const, result: '{"stdout": "PORT     STATE SERVICE\\n22/tcp   open  ssh\\n80/tcp   open  http\\n8080/tcp open  http-proxy"}' },
      { id: "te-3", tool: "read_file", step: "s2", args: { path: "/pentest/nuclei-templates/http/" }, status: "completed" as const, result: '{"response": "Found 45 templates in http/ directory"}' },
      { id: "te-4", tool: "run_pty_cmd", step: "s3", args: { command: "nmap -sC -sV -p 22,80,8080 -A 8.138.179.62" }, status: v === "active" ? "running" as const : "completed" as const, result: v === "active" ? null : '{"stdout": "PORT     STATE SERVICE  VERSION\\n22/tcp   open  ssh      OpenSSH 8.2p1\\n80/tcp   open  http     nginx 1.18.0\\n8080/tcp open  http     Apache Tomcat 9.0.41"}' },
    ];

    for (const te of toolExecs) {
      state.addToolExecutionBlock(sid, {
        requestId: te.id,
        toolName: te.tool,
        args: te.args,
        source: undefined,
      });
      useStore.setState((s) => {
        const tl = s.timelines[sid];
        if (!tl) return;
        const block = tl.find((b) => b.type === "ai_tool_execution" && b.data.requestId === te.id);
        if (block && block.type === "ai_tool_execution") {
          block.data.planStepId = te.step;
          block.data.status = te.status;
          if (te.result) block.data.result = te.result;
        }
      });
    }

    state.setDetailViewMode(sid, "tool-detail");

    if (v === "retired") {
      useStore.setState((s) => {
        if (s.sessions[sid]) {
          s.sessions[sid].retiredPlans = [{
            plan: {
              version: 0,
              updated_at: new Date(now - 3600000).toISOString(),
              summary: { total: 4, completed: 2, in_progress: 0, failed: 0 },
              steps: [
                { id: "old-1", step: "DNS resolution and subdomain enumeration", status: "completed" as const },
                { id: "old-2", step: "Port scanning", status: "completed" as const },
                { id: "old-3", step: "HTTP service probing", status: "cancelled" as const },
                { id: "old-4", step: "Vulnerability scanning", status: "pending" as const },
              ],
            },
            messageId: planMsgId,
            retiredAt: new Date(now - 3600000).toISOString(),
          }];
        }
      });
    }

    if (v === "done") {
      state.addConversationMessage(convId, {
        id: `mock-final-${now}`,
        role: "assistant",
        content: "## Security Assessment Complete\n\nAll 5 steps have been completed successfully. Key findings:\n- 3 open ports detected (22, 80, 8080)\n- Apache Tomcat 9.0.41 on port 8080 (known CVEs)\n- 2 medium-severity vulnerabilities found\n\nSee the detailed report for remediation recommendations.",
        timestamp: now,
      });
    }

    console.log(`[__mockPlan] Injected full "${v}" scenario into session ${sid}, convId=${convId}`);
  };
}
