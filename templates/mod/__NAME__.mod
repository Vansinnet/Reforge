return {
    run = function()
        fassert(rawget(_G, "new_mod"), "`{{NAME}}` failed loading DMF.")
        new_mod("{{NAME}}", {
            mod_script = "{{NAME}}/scripts/mods/{{NAME}}/{{NAME}}",
            mod_data = "{{NAME}}/scripts/mods/{{NAME}}/{{NAME}}_data",
            mod_localization = "{{NAME}}/scripts/mods/{{NAME}}/{{NAME}}_localization",
        })
    end,
    packages = {},
}
