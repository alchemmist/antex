export type AntexConfigValue = string | number | boolean | AntexConfigValue[] | AntexConfigObject;

export type AntexConfigObject = { [key: string]: AntexConfigValue };

export type AntexOptions = {
  antexPathOverride?: string;
  baseUrl?: string;
  apiKey?: string;
  /**
   * Additional `--config key=value` overrides to pass to the Antex CLI.
   *
   * Provide a JSON object and the SDK will flatten it into dotted paths and
   * serialize values as TOML literals so they are compatible with the CLI's
   * `--config` parsing.
   */
  config?: AntexConfigObject;
  /**
   * Raw `--config key=value` overrides to pass unchanged to the Antex CLI after
   * structured configuration and before SDK-managed or thread-specific overrides.
   */
  configOverrides?: string[];
  /**
   * Environment variables passed to the Antex CLI process. When provided, the SDK
   * will not inherit variables from `process.env`.
   */
  env?: Record<string, string>;
};
