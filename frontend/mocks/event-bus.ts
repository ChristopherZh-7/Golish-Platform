/**
 * Mock event system (custom implementation for browser mode).
 *
 * A tiny listener registry that the patched `@tauri-apps/api/event` `listen`
 * routes through, plus `dispatchMockEvent` which the emit helpers
 * (`./events`) and simulations fan out through.
 */

// Auto-incrementing handler ID
let nextHandlerId = 1;

// Map of event name -> array of { handlerId, callback }
export const mockEventListeners: Map<
  string,
  Array<{ handlerId: number; callback: (event: { event: string; payload: unknown }) => void }>
> = new Map();

// Map of handler ID -> { event, callback } (for unlisten)
const handlerToEvent: Map<number, string> = new Map();

/**
 * Register an event listener with its callback
 */
export function mockRegisterListener(
  event: string,
  callback: (event: { event: string; payload: unknown }) => void
): number {
  const handlerId = nextHandlerId++;
  if (!mockEventListeners.has(event)) {
    mockEventListeners.set(event, []);
  }
  mockEventListeners.get(event)?.push({ handlerId, callback });
  handlerToEvent.set(handlerId, event);
  console.log(`[Mock Events] Registered listener for "${event}" (handler: ${handlerId})`);
  return handlerId;
}

/**
 * Unregister an event listener by handler ID
 */
export function mockUnregisterListener(handlerId: number): void {
  const eventName = handlerToEvent.get(handlerId);
  if (!eventName) return;

  handlerToEvent.delete(handlerId);
  const listeners = mockEventListeners.get(eventName);
  if (listeners) {
    const filtered = listeners.filter((l) => l.handlerId !== handlerId);
    mockEventListeners.set(eventName, filtered);
    console.log(`[Mock Events] Unregistered listener for "${eventName}" (handler: ${handlerId})`);
  }
}

/**
 * Dispatch an event to all registered listeners
 */
export function dispatchMockEvent(eventName: string, payload: unknown): void {
  const listeners = mockEventListeners.get(eventName);
  if (listeners && listeners.length > 0) {
    console.log(
      `[Mock Events] Dispatching "${eventName}" to ${listeners.length} listener(s)`,
      payload
    );
    for (const { callback } of listeners) {
      try {
        callback({ event: eventName, payload });
      } catch (e) {
        console.error(`[Mock Events] Error in listener for "${eventName}":`, e);
      }
    }
  } else {
    console.log(`[Mock Events] No listeners for "${eventName}"`, payload);
  }
}
