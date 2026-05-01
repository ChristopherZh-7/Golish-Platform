//! Transcript pretty-printer for eval results.

use golish_evals::outcome::EvalSummary;

use super::super::color;

/// Print the full agent transcript from eval results.
pub(super) fn print_transcript(summary: &EvalSummary) {
    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!("                    AGENT TRANSCRIPT");
    println!("═══════════════════════════════════════════════════════════════");

    for report in &summary.reports {
        println!();
        println!("┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("┃ Scenario: {}", report.scenario);
        println!("┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let response = &report.agent_output.response;

        if response.contains("Turn 1:") {
            let mut current_turn = String::new();
            let mut current_turn_num = 0;

            for line in response.lines() {
                if let Some(rest) = line.strip_prefix("Turn ") {
                    if let Some(colon_pos) = rest.find(':') {
                        if let Ok(num) = rest[..colon_pos].trim().parse::<u32>() {
                            if current_turn_num > 0 && !current_turn.trim().is_empty() {
                                let prompt = report
                                    .prompts
                                    .get((current_turn_num - 1) as usize)
                                    .map(|s| s.as_str());
                                print_turn(current_turn_num, prompt, &current_turn);
                            }
                            current_turn_num = num;
                            current_turn = rest[colon_pos + 1..].to_string();
                            continue;
                        }
                    }
                }
                if current_turn_num > 0 {
                    current_turn.push('\n');
                    current_turn.push_str(line);
                }
            }

            if current_turn_num > 0 && !current_turn.trim().is_empty() {
                let prompt = report
                    .prompts
                    .get((current_turn_num - 1) as usize)
                    .map(|s| s.as_str());
                print_turn(current_turn_num, prompt, &current_turn);
            }
        } else {
            println!();
            println!("┌─ Agent Response ─────────────────────────────────────────────");
            for line in response.lines() {
                println!("│ {}", line);
            }
            println!("└───────────────────────────────────────────────────────────────");
        }

        if !report.agent_output.tool_calls.is_empty() {
            println!();
            println!(
                "┌─ Tool Calls ({} total) ─────────────────────────────────────",
                report.agent_output.tool_calls.len()
            );
            for tc in &report.agent_output.tool_calls {
                let status = if tc.success { "✓" } else { "✗" };
                println!("│ {} {}", status, tc.name);
            }
            println!("└───────────────────────────────────────────────────────────────");
        }
    }

    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!();
}

fn print_turn(turn_num: u32, prompt: Option<&str>, content: &str) {
    println!();
    println!(
        "┌─ Turn {} ─────────────────────────────────────────────────────",
        turn_num
    );
    println!("│ {}:", color::cyan("User"));
    if let Some(p) = prompt {
        for line in p.lines() {
            println!("│   {}", line);
        }
    } else {
        println!("│   [prompt not available]");
    }
    println!("│");
    println!("│ {}:", color::yellow("Agent"));
    for line in content.trim().lines() {
        println!("│   {}", line);
    }
    println!("└───────────────────────────────────────────────────────────────");
}
