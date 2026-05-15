!((e) => {
  var t = {};
  function n(r) {
    if (t[r]) return t[r].exports;
    var o = (t[r] = { i: r, l: !1, exports: {} });
    return e[r].call(o.exports, o, o.exports, n), (o.l = !0), o.exports;
  }
  (n.m = e), (n.c = t), (n.p = "/static/js/");
})([
  (e, t, n) => {
    Object.defineProperty(t, "__esModule", { value: !0 });
    var r = n(1);
    fetch("/api/users", { method: "GET", headers: { Authorization: "Bearer " + r.token } }).then(
      (e) => e.json()
    );
  },
  (e, t) => {
    t.getOrder = (e) => fetch("/api/orders/" + e, { method: "GET" });
    t.deleteOrder = (e) => fetch("/api/orders/" + e, { method: "DELETE" });
    t.listProducts = () => axios.get("/api/products");
    t.login = (e, t) => axios.post("/api/login", { username: e, password: t });
    t.adminPanel = () => axios({ url: "/admin/dashboard", method: "GET", withCredentials: !0 });
  },
]);
