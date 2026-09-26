-- Offline tests for reforge.lua with a fake LuaJIT ffi and a fake DLL.
-- Run from the repository root: luajit lua/tests/test_reforge.lua

local LIB = "lua/reforge.lua"
local passed, failed = 0, 0

local function check(name, ok, detail)
	if ok then
		passed = passed + 1
		print("ok   " .. name)
	else
		failed = failed + 1
		print("FAIL " .. name .. (detail ~= nil and (": " .. tostring(detail)) or ""))
	end
end

-- Fake DLL -------------------------------------------------------------------

local function new_dll(world)
	local dll = { redirects = {}, opens = world.opens or {}, adds = {}, enabled = 1, installs = 0 }

	local function put(buf, text)
		buf.value = text
	end

	dll.Reforge_Abi = function()
		return world.abi or 1
	end
	dll.Reforge_Version = function(buf)
		put(buf, "9.9.9")
		return 1
	end
	dll.Reforge_Install = function(buf)
		dll.installs = dll.installs + 1
		if world.install_error then
			put(buf, world.install_error)
			return 0
		end
		return 1
	end
	dll.Reforge_Add = function(stock, replacement, sha, buf)
		dll.adds[#dll.adds + 1] = { stock, replacement, sha }
		if world.stock[stock] == nil then
			put(buf, stock .. " does not exist in this game version")
			return 0
		end
		if world.stock[stock] ~= sha then
			put(buf, stock .. " has changed")
			return 0
		end
		if not world.payload[replacement] then
			put(buf, "replacement " .. replacement .. " is missing")
			return 0
		end
		dll.redirects[stock] = replacement
		return 1
	end
	dll.Reforge_AddNew = function(stock, replacement, buf)
		if world.stock[stock] ~= nil then
			put(buf, stock .. " exists in the game")
			return 0
		end
		dll.redirects[stock] = replacement
		return 1
	end
	dll.Reforge_Remove = function(stock)
		local had = dll.redirects[stock] ~= nil
		dll.redirects[stock] = nil
		return had and 1 or 0
	end
	dll.Reforge_Opens = function(stock)
		return dll.opens[stock] or 0
	end
	dll.Reforge_HashFile = function(rel, buf)
		local hash = world.payload[rel]
		if hash then
			put(buf, hash)
			return 1
		end
		put(buf, "missing")
		return 0
	end
	dll.Reforge_SetEnabled = function(on)
		local previous = dll.enabled
		dll.enabled = on
		return previous
	end
	dll.Reforge_TraceEnable = function()
		return 0
	end
	dll.Reforge_TraceDump = function(buf, cap)
		if buf and cap > 5 then
			put(buf, "trace")
		end
		return 5
	end

	return dll
end

local function new_ffi(world)
	local ffi = { loads = {}, cdefs = 0 }

	ffi.new = function(_, size)
		return { value = "", size = size }
	end
	ffi.string = function(buf)
		return buf.value
	end
	ffi.cdef = function()
		ffi.cdefs = ffi.cdefs + 1
	end
	ffi.C = {
		GetModuleHandleA = function()
			return world.already_loaded and {} or nil
		end,
	}
	ffi.load = function(path)
		ffi.loads[#ffi.loads + 1] = path
		if world.missing_dll and world.missing_dll[path] then
			error("cannot load module " .. path)
		end
		world.dll = world.dll or new_dll(world)
		return world.dll
	end

	return ffi
end

-- Fake DMF mod ---------------------------------------------------------------

local modules = {}
local mods_by_name = {}

local function new_mod(name)
	local mod = { name = name, logs = {}, echoes = {}, errors = {}, commands = {} }

	function mod:get_name()
		return self.name
	end
	function mod:get_internal_data(key)
		if key == "load_order_name" then
			return self.name
		end
	end
	function mod:info(fmt, ...)
		self.logs[#self.logs + 1] = string.format(fmt, ...)
	end
	function mod:error(fmt, ...)
		self.errors[#self.errors + 1] = string.format(fmt, ...)
	end
	function mod:echo(fmt, ...)
		self.echoes[#self.echoes + 1] = string.format(fmt, ...)
	end
	function mod:command(command_name, _, fn)
		self.commands[command_name] = fn
	end
	function mod:io_dofile(path)
		return modules[path]
	end

	mods_by_name[name] = mod

	return mod
end

_G.get_mod = function(name)
	return mods_by_name[name]
end

local function fresh(world)
	_G.__reforge_instance = nil
	_G.Mods = world.no_ffi and { lua = {} } or { lua = { ffi = new_ffi(world) } }
	mods_by_name = {}

	return dofile(LIB)
end

local STOCK = { ["bundle/aa"] = "sha-aa", ["bundle/bb"] = "sha-bb", ["bundle/cc"] = "sha-cc" }

-- Scenarios ------------------------------------------------------------------

do
	local world = {
		stock = STOCK,
		payload = { ["mods/Alpha/p/aa"] = "h1", ["mods/Beta/p/aa"] = "h2", ["mods/Alpha/p/bb"] = "h3", ["mods/Alpha/p/new"] = "h4" },
	}
	local reforge = fresh(world)
	local alpha, beta = new_mod("Alpha"), new_mod("Beta")

	local a1 = reforge.register(alpha, { stock = "bundle/aa", file = "p/aa", sha256 = "sha-aa" })
	local a2 = reforge.register(alpha, { stock = "bundle/bb", file = "p/bb", sha256 = "sha-bb" })
	local b1 = reforge.register(beta, { stock = "bundle/aa", file = "p/aa", sha256 = "sha-aa", priority = 5 })
	local v1 = reforge.register(alpha, { stock = "bundle/zz", file = "p/new", virtual = true })

	check("pending before commit", reforge.state(a1) == "pending")
	check("dll not loaded before commit", #_G.Mods.lua.ffi.loads == 0)

	reforge.commit()

	check("dll loaded from first carrier", _G.Mods.lua.ffi.loads[1] == "../mods/Alpha/bin/reforge.dll", _G.Mods.lua.ffi.loads[1])
	check("higher priority wins", reforge.state(b1) == "active", reforge.state(b1))
	check("lower priority displaced", reforge.state(a1) == "displaced", reforge.state(a1))
	check("dll serves the winner", world.dll.redirects["bundle/aa"] == "mods/Beta/p/aa", world.dll.redirects["bundle/aa"])
	check("multi-argument calls reach the dll", world.dll.adds[1][3] == "sha-aa" and world.dll.adds[1][2] ~= nil)
	check("unique file active", reforge.state(a2) == "active")
	check("virtual file active", reforge.state(v1) == "active" and world.dll.redirects["bundle/zz"] == "mods/Alpha/p/new")
	check("served() helper", reforge.served("active") and reforge.served("shared") and not reforge.served("displaced"))
	check("winner reports owner", reforge.winner(a1) == "Beta")

	local lib_version, dll_version = reforge.version()
	check("version", lib_version == 1 and dll_version == "9.9.9", dll_version)

	reforge.clear(beta)
	check("clear hands the file back", reforge.state(a1) == "active" and world.dll.redirects["bundle/aa"] == "mods/Alpha/p/aa")

	world.dll.opens["bundle/cc"] = 3
	world.payload["mods/Beta/p/cc"] = "h5"
	local late = reforge.register(beta, { stock = "bundle/cc", file = "p/cc", sha256 = "sha-cc" })
	check("late registration of an opened file needs restart", reforge.state(late) == "restart_required", reforge.state(late))

	check("set_enabled returns previous", reforge.set_enabled(false) == true and world.dll.enabled == 0)

	alpha.commands.reforge()
	check("status command echoes summary", alpha.echoes[#alpha.echoes]:find("Reforge 1, DLL 9.9.9", 1, true) ~= nil, alpha.echoes[#alpha.echoes])

	local again = dofile(LIB)
	check("reload keeps shared state", again.state(a2) == "active" and _G.Mods.lua.ffi.cdefs == 2)
end

do
	local world = { stock = STOCK, payload = { ["mods/Alpha/p/aa"] = "same", ["mods/Beta/p/aa"] = "same", ["mods/Gamma/p/aa"] = "other" } }
	local reforge = fresh(world)
	local alpha, beta, gamma = new_mod("Alpha"), new_mod("Beta"), new_mod("Gamma")

	local a = reforge.register(alpha, { stock = "bundle/aa", file = "p/aa", sha256 = "sha-aa", contract = "rgb-v1" })
	local b = reforge.register(beta, { stock = "bundle/aa", file = "p/aa", sha256 = "sha-aa" })
	local g = reforge.register(gamma, { stock = "bundle/aa", file = "p/aa", sha256 = "sha-aa", contract = "rgb-v1" })

	reforge.commit()
	check("identical payload is shared", reforge.state(b) == "shared", reforge.state(b))
	check("same contract is compatible", reforge.state(g) == "compatible", reforge.state(g))
	check("first arrival wins ties", reforge.state(a) == "active")
end

do
	local world = { stock = STOCK, payload = { ["mods/Alpha/p/aa"] = "h1", ["mods/Beta/p/aa"] = "h2" } }
	local reforge = fresh(world)
	local alpha, beta = new_mod("Alpha"), new_mod("Beta")

	local a = reforge.register(alpha, { stock = "bundle/aa", file = "p/aa", sha256 = "sha-aa" })
	local b = reforge.register(beta, { stock = "bundle/aa", file = "p/aa", sha256 = "old-hash", priority = 9 })
	local bad = reforge.register(beta, { stock = "not/bundle", file = "p/x", sha256 = "x" })

	reforge.commit()
	check("refused winner falls back to next", reforge.state(a) == "active" and reforge.state(b) == "refused")
	check("refusal reason kept", (reforge.reason(b) or ""):find("has changed", 1, true) ~= nil, reforge.reason(b))
	check("bad spec refused in lua", reforge.state(bad) == "refused" and reforge.reason(bad):find("bundle/", 1, true) ~= nil)
	check("refusal logged once", #beta.logs == 2, #beta.logs)
end

do
	local world = {
		stock = STOCK,
		payload = { ["mods/Beta/p/aa"] = "h2" },
		missing_dll = { ["../mods/Alpha/bin/reforge.dll"] = true },
	}
	local reforge = fresh(world)
	local alpha, beta = new_mod("Alpha"), new_mod("Beta")

	reforge.register(alpha, { stock = "bundle/bb", file = "p/bb", sha256 = "sha-bb" })
	local b = reforge.register(beta, { stock = "bundle/aa", file = "p/aa", sha256 = "sha-aa" })

	reforge.commit()
	check("falls back to another carrier's dll", _G.Mods.lua.ffi.loads[2] == "../mods/Beta/bin/reforge.dll" and reforge.state(b) == "active")
end

do
	local world = { stock = STOCK, payload = {}, already_loaded = true }
	local reforge = fresh(world)
	local alpha = new_mod("Alpha")

	reforge.register(alpha, { stock = "bundle/aa", file = "p/aa", sha256 = "sha-aa" })
	reforge.commit()
	check("reuses an already loaded dll first", _G.Mods.lua.ffi.loads[1] == "reforge.dll")
end

do
	local world = { stock = STOCK, payload = {}, abi = 2 }
	local reforge = fresh(world)
	local alpha = new_mod("Alpha")
	local a = reforge.register(alpha, { stock = "bundle/aa", file = "p/aa", sha256 = "sha-aa" })

	reforge.commit()
	check("wrong ABI is unavailable", reforge.state(a) == "unavailable" and alpha.errors[1]:find("ABI", 1, true) ~= nil, alpha.errors[1])
end

do
	local world = { stock = STOCK, payload = {}, no_ffi = true }
	local reforge = fresh(world)
	local alpha = new_mod("Alpha")
	local a = reforge.register(alpha, { stock = "bundle/aa", file = "p/aa", sha256 = "sha-aa" })

	reforge.commit()
	check("no ffi is unavailable", reforge.state(a) == "unavailable" and alpha.errors[1]:find("ffi", 1, true) ~= nil)
end

do
	local world = { stock = STOCK, payload = { ["mods/Alpha/p/aa"] = "h1", ["mods/Alpha/p/bb"] = "h3" } }
	local reforge = fresh(world)
	local alpha = new_mod("Alpha")

	modules["Alpha/scripts/mods/Alpha/reforge_manifest"] = {
		schema = 1,
		redirects = {
			{ stock = "bundle/aa", file = "p/aa", sha256 = "sha-aa" },
			{ stock = "bundle/bb", file = "p/bb", sha256 = "sha-bb" },
		},
	}

	local handles = reforge.register_manifest(alpha, "Alpha/scripts/mods/Alpha/reforge_manifest")
	reforge.commit()
	check("manifest registers every entry", #handles == 2 and reforge.state(handles[1]) == "active" and reforge.state(handles[2]) == "active")

	local none = reforge.register_manifest(alpha, { schema = 2 })
	check("wrong manifest schema rejected", #none == 0 and alpha.errors[1] ~= nil)
end

print(string.format("\n%d passed, %d failed", passed, failed))
os.exit(failed == 0 and 0 or 1)
