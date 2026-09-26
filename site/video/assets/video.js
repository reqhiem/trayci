// Helpers shared by the three clips. A classic script that loads once per clip (three times in
// the QA reel, index.html), so it only assigns. Every helper adds tweens to the caller's paused
// timeline; nothing here runs on its own clock.
window.TV = {
  // Stage-local position of a point inside an element's box (fx/fy: 0 = left/top, 1 = right/bottom).
  // Measured once while building, never at tween time.
  at(stage, el, fx = 0.5, fy = 0.5) {
    const s = stage.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    const k = s.width / stage.offsetWidth;
    return {
      x: (r.left - s.left + r.width * fx) / k,
      y: (r.top - s.top + r.height * fy) / k,
    };
  },

  // The cursor glides through waypoints [{x, y}, ...] (first = where it is) as one eased move.
  glide(tl, cursor, path, at, duration, ease = "power2.inOut") {
    tl.to(cursor, { motionPath: { path, curviness: 1 }, duration, ease }, at);
  },

  // A press of the cursor and one ring from the click point.
  click(tl, cursor, ring, point, at) {
    tl.to(cursor, { scale: 0.86, duration: 0.08, ease: "power2.in", yoyo: true, repeat: 1 }, at);
    tl.set(ring, { x: point.x, y: point.y }, at);
    tl.fromTo(
      ring,
      { scale: 0.3, opacity: 0.75 },
      { scale: 1.45, opacity: 0, duration: 0.5, ease: "power2.out", immediateRender: false },
      at,
    );
  },

  // Swap the two states of each .roll cell: forward rolls up to the second, back rolls down to the first.
  roll(tl, cells, at, back = false, stagger = 0) {
    const d = back ? -1 : 1;
    [...cells].forEach((cell, i) => {
      const [first, second] = cell.children;
      first.dataset.layoutAllowOverlap = second.dataset.layoutAllowOverlap = ""; // they cross mid-roll
      const [out, into] = back ? [second, first] : [first, second];
      const t = at + i * stagger;
      tl.to(out, { yPercent: -100 * d, autoAlpha: 0, duration: 0.34, ease: "power2.inOut" }, t);
      tl.fromTo(
        into,
        { yPercent: 100 * d, autoAlpha: 0 },
        { yPercent: 0, autoAlpha: 1, duration: 0.34, ease: "power2.inOut", immediateRender: false },
        t,
      );
    });
  },
};
