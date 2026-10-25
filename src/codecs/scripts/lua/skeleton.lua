local PROJECT_ROOT = LUDARED.PROJECT_ROOT
local WORKSPACE = LUDARED.WORKSPACE
local SCRIPTS = LUDARED.SCRIPTS
local hash = LUDARED.sha256
local get_codec = LUDARED.codec

return {
  name = "New codec",
  description = [[
    This codec does something amazing.
    It supports:
    - Feature A
    - Feature B
    - Feature C

    Arguments:
      key   A value.

    Example
      "key=value"
  ]],
  version = 1,
  -- Codec kind:
  -- - "rebuild" reconstructs the entire file from scratch
  -- - "patch" is passed the original file and applies modifications
  kind = "rebuild",

  --- Determines whether this codec can handle the input.
  --- Possible values:
  --- - `"no"`: "I cannot decode that data meaningfully, no way."
  --- - `"possible"`: "I may extract something out of that data, maybe garbage, maybe gold."
  --- - `"likely"`: "All signs point to yes, but false positives are possible."
  --- - `"certain"`: "I know that data, I know that format, I'm in charge."
  ---@param data string Input binary data.
  ---@return "no"|"possible"|"likely"|"certain"
  can_handle = function(data)
    return "no"
  end,

  --- Generates a name for the decode operation.
  ---@param args string? Codec arguments.
  ---@return string
  decode_name = function(args)
    return "codec"
  end,

  --- Extracts artifacts from the input.
  ---@param data string Input binary data.
  ---@param args string? Codec arguments.
  ---@return { name: string, data: string }[]
  decode = function(data, args)
    -- TODO: Extract artifacts from the input.
    return {
      { name = "output.bin", data = data },
    }
  end,

  --- Reconstructs the original file from its artifacts.
  ---@param original string Original bytes (empty for rebuild codecs).
  ---@param artifacts { name: string, data: string }[] Ordered artifacts.
  ---@param args string? Codec arguments.
  ---@return string
  encode = function(original, artifacts, args)
    -- TODO: Reconstruct the original data from the artifacts.
    return artifacts[1].data
  end,
}