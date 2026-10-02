import verifyCFreeze from "./qa-pc05c-freeze";
export default function globalSetup() { verifyCFreeze(); return () => verifyCFreeze(); }
