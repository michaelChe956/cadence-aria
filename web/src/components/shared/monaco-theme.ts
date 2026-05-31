import { useEffect, useState } from "react";

type MonacoTheme = "vs" | "vs-dark";

const DARK_SCHEME_QUERY = "(prefers-color-scheme: dark)";

export function useSystemMonacoTheme(): MonacoTheme {
  const [theme, setTheme] = useState<MonacoTheme>(() => resolveSystemTheme());

  useEffect(() => {
    const matcher = getDarkSchemeMatcher();
    if (!matcher) {
      return;
    }

    const handleChange = (event: MediaQueryListEvent) => {
      setTheme(event.matches ? "vs-dark" : "vs");
    };

    matcher.addEventListener("change", handleChange);
    return () => matcher.removeEventListener("change", handleChange);
  }, []);

  return theme;
}

function resolveSystemTheme(): MonacoTheme {
  const matcher = getDarkSchemeMatcher();
  return matcher?.matches ? "vs-dark" : "vs";
}

function getDarkSchemeMatcher() {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return null;
  }
  return window.matchMedia(DARK_SCHEME_QUERY);
}
