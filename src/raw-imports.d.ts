/** Vite `?raw` imports give the file text as a string (used for the bundled notices). */
declare module '*.md?raw' {
  const text: string;
  export default text;
}

/** Vite `?url` imports give the bundled file's URL on the app origin (the Rive wasm, ADR 0021). */
declare module '*.wasm?url' {
  const url: string;
  export default url;
}

/** Vite `?url` imports of the mascot PNG fallback (UX-09b, ADR 0027). */
declare module '*.png?url' {
  const url: string;
  export default url;
}
