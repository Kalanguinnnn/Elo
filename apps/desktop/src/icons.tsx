// Minimal line icons (rounded, 1.6 stroke) in the style of macOS sidebars.
const base = { width: 17, height: 17, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 1.6, strokeLinecap: "round" as const, strokeLinejoin: "round" as const };

export const IconFolder = () => (
  <svg {...base}><path d="M3.5 7.5A2 2 0 0 1 5.5 5.5h4l2 2h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" /></svg>
);
export const IconMerge = () => (
  <svg {...base}><circle cx="7" cy="6" r="2" /><circle cx="7" cy="18" r="2" /><circle cx="17" cy="12" r="2" /><path d="M7 8v8M7 9c0 3 3 3 8 3" /></svg>
);
export const IconDevices = () => (
  <svg {...base}><rect x="3.5" y="5" width="13" height="9.5" rx="2" /><path d="M2.5 18h14" /><rect x="18" y="8" width="4" height="10" rx="1.2" /></svg>
);
export const IconPulse = () => (
  <svg {...base}><path d="M3 12h4l2.5-6 4 12L16 12h5" /></svg>
);
export const IconSliders = () => (
  <svg {...base}><path d="M5 6h9M18 6h1M5 12h3M12 12h7M5 18h11M20 18h-1" /><circle cx="16" cy="6" r="2" /><circle cx="10" cy="12" r="2" /><circle cx="18" cy="18" r="2" /></svg>
);
export const IconInfo = () => (
  <svg {...base}><circle cx="12" cy="12" r="8.5" /><path d="M12 11v5M12 8h.01" /></svg>
);
export const IconChevronLeft = () => (
  <svg {...base} width={14} height={14}><path d="M15 6l-6 6 6 6" /></svg>
);
export const IconRepo = () => (
  <svg {...base}><path d="M6 4.5h10.5a1.5 1.5 0 0 1 1.5 1.5v12.5H7.5A1.5 1.5 0 0 1 6 17z" /><path d="M6 17a1.5 1.5 0 0 1 1.5-1.5H18" /></svg>
);
