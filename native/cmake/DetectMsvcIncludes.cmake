# Probe MSVC directly: its detached output uses the Windows ANSI code page.
# Ninja runs compilers in a UTF-8 console; normalize the prefix to UTF-8.
file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/include-prefix.c" "#include <stddef.h>\n")
execute_process(
    COMMAND "${CMAKE_C_COMPILER}" /nologo /showIncludes /EP
            "${CMAKE_CURRENT_BINARY_DIR}/include-prefix.c"
    OUTPUT_VARIABLE include_output
    ERROR_VARIABLE include_error
    RESULT_VARIABLE include_status
    ENCODING ANSI)
if(NOT include_status EQUAL 0)
    message(FATAL_ERROR "Cannot detect MSVC include diagnostics: ${include_error}")
endif()
string(APPEND include_output "\n${include_error}")
string(REGEX MATCH "[^\r\n]*: +[A-Za-z]:[/\\\\]" include_prefix "${include_output}")
if(NOT include_prefix)
    message(FATAL_ERROR "Cannot identify the /showIncludes prefix in MSVC output.")
endif()
string(LENGTH "${include_prefix}" prefix_length)
math(EXPR prefix_length "${prefix_length} - 3")
string(SUBSTRING "${include_prefix}" 0 ${prefix_length} CMAKE_CL_SHOWINCLUDES_PREFIX)
