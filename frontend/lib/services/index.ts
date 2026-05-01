/**
 * Services Layer — domain-level business logic and backend communication.
 *
 * Each service wraps transport-level calls with caching, deduplication,
 * and typed error handling. Components should import from here
 * rather than calling invoke() directly.
 *
 * Usage:
 *   import { aiService, pentestService, settingsService } from "@/lib/services";
 *   const tools = await pentestService.scanTools();
 */

import * as aiService from "./ai.service";
import * as pentestService from "./pentest.service";
import * as settingsService from "./settings.service";

export { aiService, pentestService, settingsService };
