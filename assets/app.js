// Live updates: the server pushes re-rendered fragments over SSE and this
// swaps them in by id. Without JavaScript the page still works; it just
// does not refresh itself or filter.
(() => {
  const version = document.body.dataset.version || "0";
  const source = new EventSource("/events?v=" + encodeURIComponent(version));
  const box = document.querySelector(".search input");

  // Typing narrows the cards to the ones whose name matches.
  const filter = () => {
    const text = box ? box.value.trim().toLowerCase() : "";
    for (const card of document.querySelectorAll(".svc")) {
      card.hidden = !!text && !card.dataset.name.toLowerCase().includes(text);
    }
    for (const group of document.querySelectorAll("main .group")) {
      group.hidden = !group.querySelector(".svc:not([hidden])");
    }
  };

  source.addEventListener("fragment", (event) => {
    const split = event.data.indexOf("\n");
    const target = document.getElementById(event.data.slice(0, split));
    if (!target) return;
    target.outerHTML = event.data.slice(split + 1);
    if (box && box.value) filter();
  });
  source.addEventListener("reload", () => location.reload());
  source.addEventListener("open", () => document.body.classList.remove("offline"));
  source.addEventListener("error", () => document.body.classList.add("offline"));

  if (!box) return;
  box.addEventListener("input", filter);
  box.form.addEventListener("submit", (event) => {
    // One card left: Enter opens it. Otherwise the text goes to the search engine.
    const left = document.querySelectorAll(".svc:not([hidden])");
    if (box.value.trim() && left.length === 1 && left[0].dataset.href) {
      event.preventDefault();
      window.open(left[0].dataset.href, "_blank", "noreferrer");
    }
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "/" && document.activeElement !== box) {
      event.preventDefault();
      box.focus();
    } else if (event.key === "Escape" && document.activeElement === box) {
      box.value = "";
      filter();
    }
  });
})();
