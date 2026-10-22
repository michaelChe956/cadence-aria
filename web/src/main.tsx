import React from "react";
import ReactDOM from "react-dom/client";
import { AppRouter } from "./router";
import "./styles.css";

export { AppShell } from "./app-shell";

const root = document.getElementById("root");

if (root) {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <AppRouter />
    </React.StrictMode>,
  );
}
