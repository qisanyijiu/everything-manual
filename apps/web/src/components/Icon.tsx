import type { CSSProperties } from "react";

const paths = {
  book: "M4 4h6a3 3 0 0 1 3 3v14a4 4 0 0 0-4-2H4V4Zm16 0h-4a3 3 0 0 0-3 3v14a4 4 0 0 1 4-2h3V4Z",
  grid: "M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z",
  activity: "M3 12h4l3-8 4 16 3-8h4",
  settings: "M4 7h16M4 17h16M8 4v6M16 14v6",
  arrow: "M5 12h14m-5-5 5 5-5 5",
  plus: "M12 5v14M5 12h14",
  search: "M21 21l-5-5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0",
  cube: "m12 3 9 5v9l-9 5-9-5V8l9-5Zm0 10 9-5M12 13 3 8m9 5v9M7.5 5.5l9 5",
  file: "M14 3H5v18h14V8l-5-5Zm0 0v5h5M8 12h8M8 16h6",
  shield: "m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Zm-4 9 3 3 5-6",
  logout: "M9 4H4v16h5M9 12h12m-4-4 4 4-4 4",
  chevron: "m9 5 7 7-7 7",
  clock: "M12 8v5l3 2M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0",
} as const;

export function Icon({ name, size = 20, style }: { name: keyof typeof paths; size?: number; style?: CSSProperties }) {
  return <svg className="icon" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" style={style}><path d={paths[name]} /></svg>;
}

/** Local vector artwork: a manual and an exploded object, rather than a pretend product photo. */
export function ManualArtwork() {
  return (
    <svg className="manual-artwork" viewBox="0 0 380 260" fill="none" aria-hidden="true">
      <circle cx="220" cy="132" r="105" stroke="currentColor" strokeOpacity=".09" />
      <circle cx="220" cy="132" r="78" stroke="currentColor" strokeOpacity=".08" strokeDasharray="3 6" />
      <path d="M38 216h304M63 39v181M328 39v181" stroke="currentColor" strokeOpacity=".08" />
      <g transform="translate(65 38) rotate(-9 90 90)">
        <rect x="4" y="7" width="143" height="183" rx="9" fill="#253E36" opacity=".08" />
        <rect width="143" height="183" rx="8" fill="#FFFEFA" stroke="#D5D6C8" />
        <path d="M17 0v183" stroke="#DADCCF" />
        <rect x="30" y="22" width="29" height="5" rx="2.5" fill="#D9774E" />
        <path d="M30 41h78M30 50h54" stroke="#B7BEB3" strokeWidth="3" strokeLinecap="round" />
        <path d="m49 90 29-16 29 16v32l-29 17-29-17V90Zm0 0 29 17 29-17m-29 17v32M63 82l30 16" stroke="#315247" strokeWidth="1.5" />
        <path d="M30 155h78M30 163h51" stroke="#D4D9CD" strokeWidth="3" strokeLinecap="round" />
      </g>
      <path d="m194 154 56-32 57 32-57 33-56-33Z" fill="#B7C4B3" stroke="#456255" />
      <path d="M194 154v17l56 33 57-33v-17l-57 33-56-33Z" fill="#E0E6D8" stroke="#456255" />
      <path d="M250 187v17" stroke="#456255" />
      <path d="M210 107v42m81-42v42m-41-65v38" stroke="#7D9484" strokeDasharray="3 4" />
      <path d="m194 95 56-32 57 32-57 33-56-33Z" fill="#DDE7D5" stroke="#456255" />
      <path d="M194 95v16l56 33 57-33V95l-57 33-56-33Z" fill="#F1F3E8" stroke="#456255" />
      <path d="M250 128v16m-24-49 24-14 24 14-24 14-24-14Z" stroke="#456255" />
      <circle cx="306" cy="69" r="17" fill="#D9774E" />
      <path d="m299 69 5 5 9-10" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M318 152h23m-23 0-6 5" stroke="#A2AE9E" />
      <circle cx="344" cy="152" r="3" fill="#D9774E" />
      <text x="237" y="235" fill="#64786B" fontSize="9" letterSpacing="2" fontFamily="monospace">FIG. 01 / EXPLORE</text>
    </svg>
  );
}
