import React from "react";
import ReactDOM from "react-dom/client";
import Entry from './portal/Entry';
import "./App.css";

import { isTauri } from "./utils/env";
// 启动时显式调用 Rust 命令显示窗口
// 配合 visible:false 使用，解决启动黑屏问题
if (isTauri()) {
  import("@tauri-apps/api/core").then(({ invoke }) => {
    invoke("show_main_window").catch(console.error);
  });
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Entry />

  </React.StrictMode>,
);
