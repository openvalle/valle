// ShaderErrorHandler's virtual interface from Skia include/gpu/ShaderErrorHandler.h.
/*
Copyright 2021 Google LLC
Copyright (c) 2011 Google Inc. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

  * Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.

  * Redistributions in binary form must reproduce the above copyright
    notice, this list of conditions and the following disclaimer in
    the documentation and/or other materials provided with the
    distribution.

  * Neither the name of the copyright holder nor the names of its
    contributors may be used to endorse or promote products derived
    from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
*/
// Kept in sync with the pinned skia-bindings 0.153.3. No Skia implementation or
// private headers are needed; rust-skia does not yet wrap this public callback.
namespace skgpu {
class ShaderErrorHandler {
public:
    virtual ~ShaderErrorHandler() = default;
    virtual void compileError(const char*, const char*) {}
    virtual void compileError(const char* shader, const char* errors, bool) {
        this->compileError(shader, errors);
    }
protected:
    ShaderErrorHandler() = default;
    ShaderErrorHandler(const ShaderErrorHandler&) = delete;
    ShaderErrorHandler& operator=(const ShaderErrorHandler&) = delete;
};
}

using Callback = void (*)(void*, const char*);
class ValleShaderErrorHandler final : public skgpu::ShaderErrorHandler {
    void* context;
    Callback callback;
public:
    ValleShaderErrorHandler(void* context, Callback callback)
        : context(context), callback(callback) {}
    void compileError(const char*, const char* errors) override {
        callback(context, errors);
    }
};

extern "C" void* valle_shader_errors_new(void* context, Callback callback) {
    return new ValleShaderErrorHandler(context, callback);
}
extern "C" void valle_shader_errors_delete(void* handler) {
    delete static_cast<ValleShaderErrorHandler*>(handler);
}
