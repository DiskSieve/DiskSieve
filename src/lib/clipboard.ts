/**
 * Copies `text` to the clipboard. Uses the async Clipboard API and falls back to a hidden
 * textarea + `execCommand("copy")` where the WebView denies clipboard-write permission.
 */
export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    // Fall through to the legacy path.
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.opacity = "0";
  document.body.appendChild(area);
  area.select();
  const ok = document.execCommand("copy");
  area.remove();
  if (!ok) throw new Error("The clipboard is not available");
}
