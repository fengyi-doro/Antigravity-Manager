import { lazy, Suspense, useEffect, useState } from "react";
import { isTauri } from "../utils/env";
const Admin = lazy(() => import("../App"));
const Portal = lazy(() => import("./Portal"));

export default function Entry() {
  const native = isTauri();
  const admin =
    native ||
    window.location.pathname === "/admin" ||
    window.location.pathname.startsWith("/admin/");
  const [checked, setChecked] = useState(native || !admin);
  useEffect(() => {
    if (!admin || native) return;
    const controller = new AbortController();
    fetch("/portal/api/me", {
      credentials: "same-origin",
      signal: controller.signal,
    })
      .then((response) => {
        if (response.ok) window.location.replace("/console");
        else setChecked(true);
      })
      .catch(() => {
        if (!controller.signal.aborted) setChecked(true);
      });
    return () => controller.abort();
  }, [admin, native]);
  return (
    <Suspense fallback={<p style={{ padding: 32 }}>正在加载…</p>}>
      {checked && (admin ? <Admin /> : <Portal />)}
    </Suspense>
  );
}
