// Live updates: the server pushes re-rendered fragments over SSE and this
// swaps them in by id. Without JavaScript the page still works; it just
// does not refresh itself.
(() => {
  const version = document.body.dataset.version || "0";
  const source = new EventSource("/events?v=" + encodeURIComponent(version));
  source.addEventListener("fragment", (event) => {
    const split = event.data.indexOf("\n");
    const target = document.getElementById(event.data.slice(0, split));
    if (target) target.outerHTML = event.data.slice(split + 1);
  });
  source.addEventListener("reload", () => location.reload());
})();
