# Third-party options and directory flags stay inside this subdirectory.
set(CMAKE_POLICY_VERSION_MINIMUM 3.5)
set(SRT_SOURCE_DIR "" CACHE PATH "Extracted SRT source directory")
set(MBEDTLS_SOURCE_DIR "" CACHE PATH "Extracted Mbed TLS full release directory")
if(NOT EXISTS "${SRT_SOURCE_DIR}/srtcore/srt.h" OR
   NOT EXISTS "${MBEDTLS_SOURCE_DIR}/framework/CMakeLists.txt")
    message(FATAL_ERROR "Dependency sources are missing. Run build.ps1 to restore them.")
endif()

set(ENABLE_PROGRAMS OFF CACHE BOOL "" FORCE)
set(ENABLE_TESTING OFF CACHE BOOL "" FORCE)
set(MBEDTLS_FATAL_WARNINGS OFF CACHE BOOL "" FORCE)
set(USE_STATIC_MBEDTLS_LIBRARY ON CACHE BOOL "" FORCE)
set(USE_SHARED_MBEDTLS_LIBRARY OFF CACHE BOOL "" FORCE)
add_subdirectory("${MBEDTLS_SOURCE_DIR}" mbedtls EXCLUDE_FROM_ALL)

set(ENABLE_APPS OFF CACHE BOOL "" FORCE)
set(ENABLE_UNITTESTS OFF CACHE BOOL "" FORCE)
set(ENABLE_SHARED OFF CACHE BOOL "" FORCE)
set(ENABLE_STATIC ON CACHE BOOL "" FORCE)
set(ENABLE_STDCXX_SYNC ON CACHE BOOL "" FORCE)
set(ENABLE_ENCRYPTION ON CACHE BOOL "" FORCE)
set(USE_ENCLIB mbedtls CACHE STRING "" FORCE)
set(SSL_LIBRARY_DIRS "${CMAKE_CURRENT_BINARY_DIR}/mbedtls/library")
set(SSL_INCLUDE_DIRS "${MBEDTLS_SOURCE_DIR}/include")
set(SSL_LIBRARIES mbedcrypto)
add_subdirectory("${SRT_SOURCE_DIR}" srt EXCLUDE_FROM_ALL)
target_compile_options(srt_virtual PRIVATE /utf-8)

add_library(bondriver_srt_sdk INTERFACE)
target_link_libraries(bondriver_srt_sdk INTERFACE srt_static)
target_include_directories(bondriver_srt_sdk INTERFACE
    "${SRT_SOURCE_DIR}/srtcore" "${SRT_SOURCE_DIR}/common" "${CMAKE_CURRENT_BINARY_DIR}/srt")
