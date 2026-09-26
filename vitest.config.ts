import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // A git worktree under `.claude/` brings its own node_modules and its own copy of
    // every test in the tree; without this, `pnpm test` runs the dependencies' test suites.
    // The agent skills in `.agents/` carry their own tests, which are not this app's.
    exclude: ["**/node_modules/**", "dist/**", ".agents/**", ".claude/**"],
  },
});
