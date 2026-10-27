import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { DesignPlayground } from "./components/DesignPlayground";
import Dashboard from "./components/Dashboard";
import ConversationDetails from "./components/ConversationDetails";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>{new URLSearchParams(window.location.search).has("conversation") ? <ConversationDetails threadId={new URLSearchParams(window.location.search).get("conversation")??""}/> : new URLSearchParams(window.location.search).has("dashboard") ? <Dashboard /> : new URLSearchParams(window.location.search).has("designer") ? <DesignPlayground /> : <App />}</React.StrictMode>,
);
