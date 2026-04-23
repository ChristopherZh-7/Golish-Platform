import { ThemeRegistry } from "../registry";
import { cyberpunkTheme } from "./cyberpunk/theme";
import { daylightTheme } from "./daylight/theme";
import { deepOceanTheme } from "./deep-ocean/theme";
import { golishTheme } from "./golish/theme";
import { moonlightTheme } from "./moonlight/theme";
import { obsidianEmber } from "./obsidian-ember/theme";
import { redAlertTheme } from "./red-alert/theme";

/**
 * Register all builtin themes
 */
export function registerBuiltinThemes(): void {
  ThemeRegistry.register("golish", golishTheme, true);
  ThemeRegistry.register("obsidian-ember", obsidianEmber, true);
  ThemeRegistry.register("cyberpunk", cyberpunkTheme, true);
  ThemeRegistry.register("deep-ocean", deepOceanTheme, true);
  ThemeRegistry.register("red-alert", redAlertTheme, true);
  ThemeRegistry.register("moonlight", moonlightTheme, true);
  ThemeRegistry.register("daylight", daylightTheme, true);
}
