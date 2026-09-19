# Example plugins

Three plugins that do something worth having, each written only against what
`docs/plugins.md` documents. Copy one into your config and require it:

```sh
mkdir -p ~/.config/neotmux/lua/plugin
cp examples/clock.lua ~/.config/neotmux/lua/plugin/
echo "require('plugin.clock')" >> ~/.config/neotmux/init.lua
```

Delete that `require` line and the plugin is gone. There is nothing else to undo.

| file | what it shows |
|---|---|
| `clock.lua` | the status line, and reading config a user set |
| `prefix.lua` | `on_key`, modal state, and driving panes |
| `pane_labels.lua` | drawing into a specific pane, and per-pane geometry |
