import type { XdebugLaunchHints } from "./types";

/** `.vscode/launch.json` content for a site, mirroring what the backend writes. */
export function vscodeLaunchSnippet(h: XdebugLaunchHints): string {
  return JSON.stringify(
    {
      version: "0.2.0",
      configurations: [
        {
          name: "Listen for Xdebug (DevStack)",
          type: "php",
          request: "launch",
          port: h.port,
          pathMappings: { [h.phpstorm.remotePath]: "${workspaceFolder}" },
        },
      ],
    },
    null,
    2,
  );
}

export function phpstormSteps(h: XdebugLaunchHints): string[] {
  const p = h.phpstorm;
  return [
    `Settings > PHP > Servers: add "${p.serverName}" (host ${p.host}, port ${p.port}, debugger Xdebug).`,
    `Enable path mappings: ${p.localPath} -> ${p.remotePath}.`,
    `Settings > PHP > Debug: Xdebug port ${h.port}, then start listening for PHP debug connections.`,
    `IDE key: ${p.ideKey}.`,
  ];
}
