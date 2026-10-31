import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyMotion } from "./motion";
import "./styles.css";

// Before the first paint, so performance mode never flashes an animation.
applyMotion();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
