/**
 * Task mode event handlers.
 *
 * Handles: task_progress, subtask_created, subtask_completed.
 * These events are emitted during PentAGI-style automated task execution.
 */

import { logger } from "@/lib/logger";
import type { EventHandler } from "./types";

let taskEventCounter = 0;
function nextId() {
  return `task-evt-${++taskEventCounter}-${Date.now()}`;
}

export const handleTaskProgress: EventHandler<{
  type: "task_progress";
  task_id: string;
  status: string;
  message: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  logger.info(
    `[Task ${event.status}] ${event.message} (task: ${event.task_id})`
  );
  const state = ctx.getState();
  state.addAgentMessage(ctx.sessionId, {
    id: nextId(),
    sessionId: ctx.sessionId,
    role: "system",
    content: `**[Task ${event.status}]** ${event.message}`,
    timestamp: new Date().toISOString(),
  });
};

export const handleSubtaskCreated: EventHandler<{
  type: "subtask_created";
  task_id: string;
  subtask_id: string;
  title: string;
  agent: string | null;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  const agentLabel = event.agent ?? "auto";
  logger.info(
    `[Subtask Created] ${event.title} (agent: ${agentLabel}, id: ${event.subtask_id})`
  );
  const state = ctx.getState();
  state.addAgentMessage(ctx.sessionId, {
    id: nextId(),
    sessionId: ctx.sessionId,
    role: "system",
    content: `**Subtask created:** ${event.title} *(${agentLabel})*`,
    timestamp: new Date().toISOString(),
  });
};

export const handleSubtaskCompleted: EventHandler<{
  type: "subtask_completed";
  task_id: string;
  subtask_id: string;
  title: string;
  result: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  logger.info(`[Subtask Completed] ${event.title}`);
  const state = ctx.getState();
  state.addAgentMessage(ctx.sessionId, {
    id: nextId(),
    sessionId: ctx.sessionId,
    role: "system",
    content: `**Subtask completed:** ${event.title}\n\n${event.result}`,
    timestamp: new Date().toISOString(),
  });
};

export const handleSubtaskWaitingForInput: EventHandler<{
  type: "subtask_waiting_for_input";
  task_id: string;
  subtask_id: string;
  title: string;
  prompt: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  logger.info(`[Subtask Waiting] ${event.title}: ${event.prompt}`);
  const state = ctx.getState();
  state.addAgentMessage(ctx.sessionId, {
    id: nextId(),
    sessionId: ctx.sessionId,
    role: "system",
    content: `**Waiting for input:** ${event.title}\n\n${event.prompt}`,
    timestamp: new Date().toISOString(),
  });
};

export const handleSubtaskUserInput: EventHandler<{
  type: "subtask_user_input";
  task_id: string;
  subtask_id: string;
  input: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  logger.info(`[User Input] ${event.input}`);
  const state = ctx.getState();
  state.addAgentMessage(ctx.sessionId, {
    id: nextId(),
    sessionId: ctx.sessionId,
    role: "system",
    content: `**User provided input:** ${event.input}`,
    timestamp: new Date().toISOString(),
  });
};

export const handleTaskResumed: EventHandler<{
  type: "task_resumed";
  task_id: string;
  subtask_index: number;
  total_subtasks: number;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  logger.info(
    `[Task Resumed] from subtask ${event.subtask_index}/${event.total_subtasks}`
  );
  const state = ctx.getState();
  state.addAgentMessage(ctx.sessionId, {
    id: nextId(),
    sessionId: ctx.sessionId,
    role: "system",
    content: `**Task resumed** from subtask ${event.subtask_index}/${event.total_subtasks}`,
    timestamp: new Date().toISOString(),
  });
};

export const handleEnricherResult: EventHandler<{
  type: "enricher_result";
  task_id: string;
  subtask_id: string;
  context_added: string;
  session_id: string;
  seq?: number;
}> = (event, ctx) => {
  logger.info(`[Enricher] Added context: ${event.context_added}`);
  const state = ctx.getState();
  state.addAgentMessage(ctx.sessionId, {
    id: nextId(),
    sessionId: ctx.sessionId,
    role: "system",
    content: `**Enricher:** ${event.context_added}`,
    timestamp: new Date().toISOString(),
  });
};
