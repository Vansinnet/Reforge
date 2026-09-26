local mod = get_mod("{{NAME}}")

-- Reforge serves this mod's payload files in place of the game's own files.
-- Edit reforge.json and run `reforge build` to change what is replaced.
local reforge = mod:io_dofile("{{NAME}}/scripts/mods/{{NAME}}/reforge")
local handles = reforge.register_manifest(mod, "{{NAME}}/scripts/mods/{{NAME}}/reforge_manifest")

mod.resources_ready = false

mod.on_all_mods_loaded = function()
    reforge.commit()

    local served, restart = 0, false

    for i = 1, #handles do
        local handle = handles[i]
        local state = reforge.state(handle)

        if reforge.served(state) then
            served = served + 1
        else
            restart = restart or state == "restart_required"
            mod:info("%s stays stock: %s %s", handle.stock, state, reforge.reason(handle) or "")
        end
    end

    mod.resources_ready = #handles > 0 and served == #handles

    if restart then
        mod:echo(mod:localize("restart_required"))
    elseif #handles > 0 and not mod.resources_ready then
        mod:echo(mod:localize("resources_missing"))
    end
end
