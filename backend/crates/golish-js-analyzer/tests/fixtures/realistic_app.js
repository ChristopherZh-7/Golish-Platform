// Realistic mixed-pattern fixture: fetch / axios / $.ajax / new Request
// covering all 5 patterns + 3 url_kind shapes.
// This is intentionally NOT minified — see `minified_webpack.js` for that.

const API_BASE = "https://api.example.com";

// ─── plain fetch (literal) ──────────────────────────────────────────────
fetch("/api/me", { method: "GET", headers: { Authorization: "Bearer " + token } });
fetch("/api/health");

// ─── fetch with concatenation ───────────────────────────────────────────
fetch("/api/orders/" + orderId, { method: "DELETE" });
fetch("/api/users/" + userId);

// ─── fetch with template literal ────────────────────────────────────────
fetch(`/api/users/${id}/posts`, { method: "GET" });
fetch(`/api/items/${itemId}`, { method: "PUT" });

// ─── axios verb helpers ─────────────────────────────────────────────────
axios.get("/api/products");
axios.post("/api/orders", payload);
axios.delete("/api/items/12345");

// ─── axios config object ────────────────────────────────────────────────
axios({
  url: "/api/login",
  method: "POST",
  data: { username, password },
  withCredentials: true,
});

// ─── jQuery $.ajax (legacy `type` key) ──────────────────────────────────
$.ajax({
  url: "/legacy/admin/users",
  type: "POST",
  data: { name: "admin" },
});

// ─── new Request ────────────────────────────────────────────────────────
const req = new Request("/api/v2/data/507f1f77bcf86cd799439011", {
  method: "PATCH",
  headers: { "X-Token": apiToken },
});

// ─── noise that should NOT match (P1 noise filter handles these) ────────
// "fetch('/notamatch')" — inside a comment
const docs = "axios.get('/example.com/skip-me')"; // string assignment, not a call site
/* fetch('/blockcomment') */
console.log("this is just a fetch log message");
