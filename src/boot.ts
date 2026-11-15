// Entry point. A module script delays the first paint until its whole static import graph has
// loaded, which would keep the inline loader in index.html invisible. So this entry imports
// nothing: it lets the browser paint the loader first, then loads the app.
let started = false;
const start = () => {
  if (started) return;
  started = true;
  void import("./main");
};
// After the next painted frame; the timeout covers hidden/minimized windows, where frames pause.
requestAnimationFrame(() => setTimeout(start));
setTimeout(start, 100);
