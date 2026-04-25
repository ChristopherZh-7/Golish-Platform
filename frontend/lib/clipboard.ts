import { logger } from "@/lib/logger";

export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (error) {
    logger.error("Failed to copy to clipboard:", error);
    return false;
  }
}
