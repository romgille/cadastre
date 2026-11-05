# Obsidian and Syncthing

cadastre writes plain files, so any sync tool works. This page covers the
common setup: cadastre on a server, Syncthing replicating a folder of your
vault to your other devices, Obsidian reading it.

## Which folder

Give cadastre its own subfolder of the vault, for example `Infrastructure/`,
and mount that as `CADASTRE_OUT_DIR`. It owns everything it generated in
there. Notes you write in the vault outside that folder are never touched.

## Obsidian flavour

Set `CADASTRE_OBSIDIAN=true` to add front matter (`tags`, and the stack name
and container count on stack pages) and `[[wikilinks]]` between the index and
the stack pages. Leave it unset for plain Markdown with relative links.

## Syncthing

- **Ignore `.tmp`.** cadastre writes each file to `.tmp/` and renames it into
  place. Add this to the folder's `.stignore`:

  ```
  /Infrastructure/.tmp
  ```

- **Run as the same user as Syncthing.** Set `user:` in your compose file to
  the uid:gid that owns the vault folder. Find it with
  `docker exec <syncthing> id` or `ls -ln` on the folder. A mismatch shows up
  as permission errors on the first run, which is loud.
- **No churn.** A page whose content did not change is not rewritten, and
  output contains no timestamps, so a quiet system produces no sync traffic.
  Only `changelog.md` gains lines, and only when something changed.
- **Make the folder send-only on the server** if you never want edits from
  other devices to flow back. cadastre overwrites its pages anyway, so edits to
  generated pages are lost on the next run.

## Check it end to end

Start a throwaway container, wait one debounce period (30 s), and confirm the
change appears in Obsidian on another device. This is the only check that
covers the whole chain, including the mount and the sync, which cadastre
cannot see.
