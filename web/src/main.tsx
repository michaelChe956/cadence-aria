import React from "react";
import ReactDOM from "react-dom/client";
import { AuthGate } from "./api/auth";
import { AppRouter } from "./router";
import "./styles.css";

export { AppShell } from "./app-shell";

const root = document.getElementById("root");

if (root) {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <AuthGate>
        <AppRouter />
      </AuthGate>
    </React.StrictMode>,
  );
}
