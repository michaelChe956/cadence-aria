export function decodeHtmlEntitiesForDisplay(content: string) {
  if (!content.includes("&")) {
    return content;
  }

  return content
    .replace(/&quot;/g, '"')
    .replace(/&#34;/g, '"')
    .replace(/&#x22;/gi, '"')
    .replace(/&apos;/g, "'")
    .replace(/&#39;/g, "'")
    .replace(/&#x27;/gi, "'")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">");
}

export function normalizeDisplayText(content: string) {
  const decoded = decodeHtmlEntitiesForDisplay(content);
  const trimmed = decoded.trim();
  if (
    (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
    (trimmed.startsWith("[") && trimmed.endsWith("]"))
  ) {
    try {
      return JSON.stringify(JSON.parse(trimmed), null, 2);
    } catch {
      return decoded;
    }
  }

  return decoded;
}
