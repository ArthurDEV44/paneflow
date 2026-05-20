import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  // Override default ignores of eslint-config-next.
  globalIgnores([
    // Default ignores of eslint-config-next:
    ".next/**",
    "out/**",
    "build/**",
    "next-env.d.ts",
    // fumadocs-mdx generated typegen output (regenerated on every build).
    ".source/**",
  ]),
  {
    // Docs surface is built on fumadocs-core (headless); fumadocs-ui ships
    // its own Radix + Tailwind-v3 shell and would fight CossUI tokens.
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["fumadocs-ui", "fumadocs-ui/*"],
              message:
                "Use fumadocs-core (headless) with CossUI components instead. See tasks/prd-fumadocs-docs.md.",
            },
          ],
        },
      ],
    },
  },
]);

export default eslintConfig;
