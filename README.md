# pac_proxy

Few non browser based applications support PAC files in order to choose proxy.
This can be problematic in corporate environnements with roaming devices, because the proxies won't always be correct.

This is an HTTP proxy that chooses the correct distant host (either another Proxy or a Direct connection) according to the PAC file.

## Usage

Download a PAC file and run the proxy with `-f <pac_file>`.
