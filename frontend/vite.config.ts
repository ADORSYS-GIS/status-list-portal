import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { playwright } from '@vitest/browser-playwright';
import { defineConfig } from 'vitest/config';

/** Where the portal's backend listens while the frontend's dev server runs. */
const API_HOST = process.env.API_HOST ?? 'http://127.0.0.1:8080';

export default defineConfig({
  plugins: [
    sveltekit({
      preprocess: [vitePreprocess()],
      adapter: adapter({
        // The backend serves this page for every path that the frontend routes itself.
        fallback: '200.html',
        // `.br` and `.gz` copies, which the backend serves to browsers that accept them.
        precompress: true,
      }),
      csp: {
        // The build hashes its inline scripts into a `<meta>` policy, which the backend
        // sends as a header.
        mode: 'hash',
        directives: {
          'default-src': ['self'],
          'script-src': ['self'],
          'style-src': ['self'],
          // SvelteKit's route announcer, which tells screen readers about page changes,
          // positions itself with a `style` attribute. Style elements stay `self` only.
          'style-src-attr': ['unsafe-inline'],
          'img-src': ['self'],
          'font-src': ['self'],
          'connect-src': ['self'],
          'object-src': ['none'],
          'base-uri': ['self'],
          'form-action': ['self'],
        },
      },
    }),
  ],

  build: {
    // Inlined assets would be `data:` URLs, which the policy does not allow.
    assetsInlineLimit: 0,
  },

  server: {
    proxy: {
      '/api': { target: API_HOST },
    },
  },

  test: {
    expect: { requireAssertions: true },
    projects: [
      {
        extends: './vite.config.ts',
        test: {
          name: 'browser',
          browser: {
            enabled: true,
            provider: playwright(),
            instances: [{ browser: 'chromium', headless: true }],
          },
          include: ['src/**/*.svelte.test.ts'],
        },
      },
      {
        extends: './vite.config.ts',
        test: {
          name: 'node',
          environment: 'node',
          include: ['src/**/*.test.ts'],
          exclude: ['src/**/*.svelte.test.ts'],
        },
      },
    ],
  },
});
