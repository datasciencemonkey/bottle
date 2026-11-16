/** Vite `?raw` imports give the file text as a string (used for the bundled notices). */
declare module '*.md?raw' {
  const text: string;
  export default text;
}
