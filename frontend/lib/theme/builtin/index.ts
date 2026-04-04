import { ThemeRegistry } from "../registry";
import { obsidianEmber } from "./obsidian-ember/theme";
import { golishTheme } from "./golish/theme";

/**
 * Register all builtin themes
 */
export function registerBuiltinThemes(): void {
  ThemeRegistry.register("golish", golishTheme, true);
  ThemeRegistry.register("obsidian-ember", obsidianEmber, true);
}
