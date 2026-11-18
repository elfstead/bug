#include "bug.h"

#include <AK/HashMap.h>
#include <AK/JsonArray.h>
#include <AK/JsonObject.h>
#include <AK/LexicalPath.h>
#include <AK/Utf16String.h>
#include <LibCore/EventLoop.h>
#include <LibCore/Promise.h>
#include <LibGfx/Bitmap.h>
#include <LibGfx/SharedImageBuffer.h>
#include <LibGfx/SystemTheme.h>
#include <LibMain/Main.h>
#include <LibWebCommon/HTML/SelectedFile.h>
#include <LibWebCommon/HTML/VisibilityState.h>
#include <LibWebCommon/Page/InputEvent.h>
#include <LibWebCommon/WebView/Utilities.h>
#include <LibWebView/Application.h>
#include <LibWebView/PageBackingStores.h>
#include <LibWebView/URL.h>
#include <LibWebView/Utilities.h>
#include <LibWebView/ViewImplementation.h>

#include <sys/stat.h>

#include <cstring>
#include <cmath>

namespace {
BugEvent callback;
void* callback_user;
Core::AnonymousBuffer theme;
HashMap<u64, u64> evaluation_views;
u64 next_request;

void emit(uint32_t kind, uint64_t id, StringView text = {})
{
    if (callback)
        callback(callback_user, kind, id, text.characters_without_null_termination(), text.length());
}

class View;
HashMap<u64, NonnullOwnPtr<View>> views;
View& new_view(Optional<WebView::CanonicalTraversable&> = {}, WebView::IsPrivate = WebView::IsPrivate::No);

class View final : public WebView::ViewImplementation {
public:
    AK_ALLOC_WITH_KMALLOC;
    View(Optional<WebView::CanonicalTraversable&> traversable, WebView::IsPrivate is_private)
        : ViewImplementation(is_private)
    {
        // Set the initial preference before initialize_tab creates a compositor
        // context. The public setter requires an already initialized page.
        m_presents_from_cpu_memory = true;
        on_request_file_picker = [this](Web::HTML::FileFilter const& filter, Web::HTML::AllowMultipleFiles multiple) {
            JsonArray accept;
            for (auto const& entry : filter.filters) {
                entry.visit(
                    [&](Web::HTML::FileFilter::FileType type) {
                        using Type = Web::HTML::FileFilter::FileType;
                        switch (type) {
                        case Type::Audio: accept.must_append("audio/*"sv); break;
                        case Type::Image: accept.must_append("image/*"sv); break;
                        case Type::Video: accept.must_append("video/*"sv); break;
                        }
                    },
                    [&](Web::HTML::FileFilter::MimeType const& mime) { accept.must_append(mime.value.to_utf8()); },
                    [&](Web::HTML::FileFilter::Extension const& ext) { accept.must_append(MUST(String::formatted(".{}", ext.value.to_utf8()))); });
            }
            m_picker_request = ++next_request;
            m_picker_multiple = multiple == Web::HTML::AllowMultipleFiles::Yes;
            JsonObject event;
            event.set("request"sv, m_picker_request);
            event.set("multiple"sv, m_picker_multiple);
            event.set("accept"sv, move(accept));
            m_picker_delegated = handles("file_picker"sv);
            if (m_picker_delegated)
                event.set("kind"sv, "file_picker"sv);
            emit(m_picker_delegated ? BUG_REQUEST : BUG_FILE_PICKER, view_id(), event.serialized());
        };
        on_load_start = [this] { cancel_requests(); };
        on_ready_to_paint = [this] { emit(BUG_FRAME, view_id()); };
        on_title_change = [this](auto const& title) { emit(BUG_TITLE, view_id(), title.to_utf8()); };
        on_url_change = [this](auto const& url) { emit(BUG_URL, view_id(), url.serialize()); };
        on_load_finish = [this](auto const& url) { emit(BUG_LOADED, view_id(), url.serialize()); };
        on_close = [this] {
            forget_requests();
            auto id = view_id();
            // Destruction must not occur inside a renderer callback on this view.
            Core::EventLoop::current().deferred_invoke([id] {
                Vector<u64> requests;
                for (auto const& entry : evaluation_views)
                    if (entry.value == id) requests.append(entry.key);
                for (auto request : requests) evaluation_views.remove(request);
                if (views.remove(id))
                    emit(BUG_CLOSED, id);
            });
        };
        on_new_web_view = [this](auto, auto, auto& traversable) {
            return new_view(traversable, this->is_private()).handle();
        };
        on_link_hover = [this](auto const& url) { emit(BUG_HOOK, view_id(), ByteString::formatted("link {}", url.serialize())); };
        on_web_content_crashed = [this](auto) {
            forget_requests();
            emit(BUG_ERROR, view_id(), "renderer crashed; reload to retry"sv);
        };

        on_request_alert = [this](auto const& message) {
            if (!handles("alert"sv)) {
                emit(BUG_HOOK, view_id(), ByteString::formatted("alert {}", message.to_utf8()));
                alert_closed();
                return;
            }
            JsonObject data;
            data.set("message"sv, message.to_utf8());
            add_request("alert"sv, move(data), [this](int, StringView) { alert_closed(); });
        };
        on_request_confirm = [this](auto const& message) {
            if (!handles("confirm"sv)) {
                emit(BUG_HOOK, view_id(), ByteString::formatted("confirm-dismissed {}", message.to_utf8()));
                confirm_closed(false);
                return;
            }
            JsonObject data;
            data.set("message"sv, message.to_utf8());
            add_request("confirm"sv, move(data), [this](int accept, StringView) { confirm_closed(accept); });
        };
        on_request_prompt = [this](auto const& message, auto const& initial) {
            if (!handles("prompt"sv)) {
                emit(BUG_HOOK, view_id(), ByteString::formatted("prompt-dismissed {}", message.to_utf8()));
                prompt_closed({});
                return;
            }
            JsonObject data;
            data.set("message"sv, message.to_utf8());
            data.set("default"sv, initial.to_utf8());
            add_request("prompt"sv, move(data), [this](int accept, StringView text) {
                if (accept) prompt_closed(Utf16String::from_utf8(text));
                else prompt_closed({});
            });
        };
        on_request_before_unload = [this](auto const& source, auto done) {
            if (!handles("before_unload"sv)) {
                emit(BUG_HOOK, view_id(), "before-unload-accepted"sv);
                done(true);
                return;
            }
            JsonObject data;
            data.set("source"sv, source);
            add_request("before_unload"sv, move(data), [done = move(done)](int leave, StringView) { done(leave); });
        };
        on_request_permission = [this](auto const& permission, auto done) {
            using Outcome = Web::Permissions::PromptOutcome;
            if (!handles("permission"sv)) {
                emit(BUG_HOOK, view_id(), "permission-dismissed"sv);
                done(Outcome::Dismissed);
                return;
            }
            JsonObject data;
            data.set("name"sv, Web::Permissions::to_string(permission.name));
            data.set("origin"sv, permission.top_level_origin.serialize());
            data.set("is_file"sv, permission.is_file);
            data.set("system_denied"sv, permission.kind == WebView::PermissionPromptKind::SystemDeniedNotice);
            data.set("show_os_hint"sv, permission.show_os_hint);
            add_request("permission"sv, move(data), [done = move(done)](int answer, StringView) {
                constexpr Outcome outcomes[] = { Outcome::Dismissed, Outcome::AllowOnce, Outcome::AllowAlways, Outcome::Block };
                done(outcomes[answer]);
            }, permission);
        };
        on_cancel_permission_prompt = [this](auto const& permission) {
            for (auto const& [id, request] : m_requests) {
                if (request.permission.has_value() && permission.is_same_request_as(*request.permission)) {
                    auto cancelled = id;
                    m_requests.remove(cancelled);
                    notify_cancelled(cancelled);
                    break;
                }
            }
        };
        initialize_tab(Web::HTML::VisibilityState::Visible, traversable);
        resize(800, 600, 1.0);
    }

    void resize(uint32_t width, uint32_t height, double scale)
    {
        m_size = { static_cast<int>(width), static_cast<int>(height) };
        m_device_pixel_ratio = scale;
        set_window_size({ static_cast<int>(width / scale), static_cast<int>(height / scale) });
        update_zoom();
        handle_resize();
    }

    bool handles(StringView kind) const
    {
        return m_handlers.contains_slow(kind);
    }

    bool set_handlers(StringView json)
    {
        auto parsed = JsonValue::from_string(json);
        if (parsed.is_error() || !parsed.value().is_array()) return false;
        Vector<String> handlers;
        for (auto const& entry : parsed.value().as_array().values()) {
            if (!entry.is_string()) return false;
            handlers.append(entry.as_string());
        }
        auto had_before_unload = handles("before_unload"sv);
        m_handlers = move(handlers);
        if (had_before_unload != handles("before_unload"sv))
            set_controller_handles_before_unload(!had_before_unload);
        cancel_requests(true);
        return true;
    }

    void add_request(StringView kind, JsonObject data, Function<void(int, StringView)> complete,
        Optional<WebView::PermissionPromptRequest> permission = {})
    {
        auto id = ++next_request;
        m_requests.set(id, Pending { kind, move(complete), move(permission) });
        data.set("kind"sv, kind);
        data.set("request"sv, id);
        emit(BUG_REQUEST, view_id(), data.serialized());
    }

    int reply(u64 id, StringView kind, int answer, StringView text)
    {
        auto it = m_requests.find(id);
        if (it == m_requests.end()) return 0;
        if (kind != it->value.kind || answer < 0 || answer > (kind == "permission"sv ? 3 : 1)) return -1;
        if (it->value.permission.has_value()
            && it->value.permission->kind == WebView::PermissionPromptKind::SystemDeniedNotice && answer != 0)
            return -1;
        auto pending = m_requests.take(id).release_value();
        if (pending.permission.has_value())
            did_show_permission_prompt(*pending.permission);
        pending.complete(answer, text);
        return 1;
    }

    bool has_modal_request() const
    {
        for (auto const& entry : m_requests) {
            if (entry.value.kind != "permission"sv) return true;
        }
        return false;
    }

    void notify_cancelled(u64 request) { emit(BUG_REQUEST_CANCELLED, view_id(), ByteString::number(request)); }

    void cancel_requests(bool removed_only = false)
    {
        if (!removed_only || (m_picker_delegated && !handles("file_picker"sv)))
            cancel_picker();
        auto ids = m_requests.keys();
        for (auto id : ids) {
            auto it = m_requests.find(id);
            if (it == m_requests.end() || (removed_only && handles(it->value.kind))) continue;
            auto pending = m_requests.take(id).release_value();
            notify_cancelled(id);
            pending.complete(pending.kind == "before_unload"sv, {});
        }
    }

    void forget_requests()
    {
        for (auto const& entry : m_requests) notify_cancelled(entry.key);
        m_requests.clear();
        if (m_picker_request && m_picker_delegated) notify_cancelled(m_picker_request);
        m_picker_request = 0;
    }

    void cancel_picker()
    {
        if (m_picker_request != 0) {
            if (m_picker_delegated) notify_cancelled(m_picker_request);
            reply_file_picker(m_picker_request, nullptr, 0);
        }
    }

    int reply_file_picker(u64 request, char const* const* paths, size_t count)
    {
        if (!request || request != m_picker_request)
            return 0;
        if (!m_picker_multiple && count > 1)
            return -1;
        Vector<Web::HTML::SelectedFile> files;
        for (size_t i = 0; i < count; ++i) {
            if (!paths || !paths[i])
                return -2;
            ByteString path(paths[i]);
            auto file = WebView::open_local_file_for_renderer(path);
            struct stat info {};
            if (file.is_error() || fstat(file.value().fd(), &info) != 0 || !S_ISREG(info.st_mode))
                return -2;
            files.empend(Utf16String::from_utf8(LexicalPath::basename(path)), file.release_value());
        }
        m_picker_request = 0;
        file_picker_closed(move(files));
        return 1;
    }

    size_t copy_frame(uint32_t* dest, size_t capacity, uint32_t& width, uint32_t& height)
    {
        width = height = 0;
        auto frame = backing_stores().displayed_frame();
        if (!frame.has_value() || !frame->shared_image_buffer)
            return 0;
        auto bitmap = frame->shared_image_buffer->bitmap_if_present();
        if (!bitmap)
            return 0;
        width = min(bitmap->width(), frame->size.width());
        height = min(bitmap->height(), frame->size.height());
        auto count = static_cast<size_t>(width) * height;
        if (dest && capacity >= count) {
            for (uint32_t y = 0; y < height; ++y)
                for (uint32_t x = 0; x < width; ++x)
                    dest[static_cast<size_t>(y) * width + x] = bitmap->get_pixel(x, y).value() & 0x00ffffff;
        }
        return count;
    }

private:
    Core::AnonymousBuffer system_theme() override { return theme; }
    Web::DevicePixelSize viewport_size() const override { return m_size; }
    Gfx::IntPoint to_content_position(Gfx::IntPoint p) const override { return p; }
    Gfx::IntPoint to_widget_position(Gfx::IntPoint p) const override { return p; }
    ViewImplementation* create_view_for_new_tab_or_window(WebView::IsPrivate is_private) override { return &new_view({}, is_private); }
    struct Pending {
        StringView kind; // Static request kind.
        Function<void(int, StringView)> complete;
        Optional<WebView::PermissionPromptRequest> permission;
    };
    Vector<String> m_handlers;
    HashMap<u64, Pending> m_requests;
    bool m_picker_delegated { false };
    u64 m_picker_request { 0 };
    bool m_picker_multiple { false };
    Web::DevicePixelSize m_size { 800, 600 };
};

View& new_view(Optional<WebView::CanonicalTraversable&> traversable, WebView::IsPrivate is_private)
{
    auto view = adopt_own(*new View(traversable, is_private));
    auto& ref = *view;
    auto id = ref.view_id();
    views.set(id, move(view));
    emit(BUG_OPENED, id);
    return ref;
}

class Application final : public WebView::Application {
    WEB_VIEW_APPLICATION(Application)
public:
    explicit Application(ByteString path) : WebView::Application(LexicalPath::dirname(path)) { }
    void complete_webdriver_content_command(u64 request, Web::WebDriver::Response response) override
    {
        auto view = evaluation_views.take(request);
        if (!view.has_value()) {
            WebView::Application::complete_webdriver_content_command(request, move(response));
            return;
        }
        JsonObject result;
        result.set("request"sv, request);
        if (response.is_error()) {
            JsonObject error;
            error.set("type"sv, "error"sv);
            if (response.error().error == "no such window"sv)
                error.set("name"sv, "Cancelled"sv);
            else
                error.set("name"sv, response.error().error);
            error.set("message"sv, response.error().message);
            result.set("result"sv, move(error));
        } else if (response.value().is_null()) {
            // Classic WebDriver ends the evaluation when a modal opens. The page
            // resumes after the controller replies, independently of this result.
            JsonObject error;
            error.set("type"sv, "error"sv);
            error.set("name"sv, "UserPromptOpened"sv);
            error.set("message"sv, "evaluation interrupted by a modal dialog"sv);
            result.set("result"sv, move(error));
        } else {
            result.set("result"sv, response.release_value());
        }
        emit(BUG_EVALUATED, *view, result.serialized());
    }
    void insert_clipboard_item(Web::Clipboard::SystemClipboardItem item) override
    {
        WebView::Application::insert_clipboard_item(move(item));
        emit(BUG_CLIPBOARD, 0, clipboard_text().to_utf8());
    }
private:
    bool should_coordinate_browser_process() const override { return false; }
    NonnullRefPtr<Core::Promise<ByteString>> ask_user_for_download_path(ByteString const& file) const override
    {
        emit(BUG_HOOK, 0, ByteString::formatted("download-unhandled {}", file));
        return Core::Promise<ByteString>::rejected(Error::from_string_literal("Bug download handler is not implemented"));
    }
};
OwnPtr<Application> application;
Vector<ByteString> argument_storage;
Vector<char*> argument_pointers;
Vector<StringView> argument_strings;

View* lookup(uint64_t id)
{
    auto entry = views.find(id);
    return entry == views.end() ? nullptr : entry->value.ptr();
}
}

extern "C" {
uint32_t bug_abi_version() { return BUG_ABI_VERSION; }

int bug_init(char const* binary, char const* profile, BugEvent handler, void* user)
{
    if (application || !binary || !profile || !handler)
        return 0;
    callback = handler;
    callback_user = user;
    argument_storage = { ByteString(binary), "--profile-path", ByteString(profile), "--force-cpu-painting", "about:blank" };
    argument_pointers.clear();
    argument_strings.clear();
    for (auto& argument : argument_storage) {
        argument_pointers.append(const_cast<char*>(argument.characters()));
        argument_strings.append(argument.view());
    }
    auto argc = static_cast<int>(argument_pointers.size());
    argument_pointers.append(nullptr);
    auto result = Application::create({ argc, argument_pointers.data(), argument_strings.span() }, ByteString(binary));
    if (result.is_error()) {
        emit(BUG_ERROR, 0, ByteString::formatted("{}", result.error()));
        return 0;
    }
    application = result.release_value();
    auto path = LexicalPath::join(WebView::s_ladybird_resource_root, "themes"sv, "Default.ini"sv);
    auto loaded_theme = Gfx::load_system_theme(path.string());
    if (loaded_theme.is_error()) {
        emit(BUG_ERROR, 0, ByteString::formatted("{}", loaded_theme.error()));
        application.clear();
        return 0;
    }
    theme = loaded_theme.release_value();
    return 1;
}

void bug_shutdown()
{
    views.clear();
    evaluation_views.clear();
    application.clear();
    callback = nullptr;
    callback_user = nullptr;
}

void bug_pump()
{
    if (application)
        Core::EventLoop::current().pump(Core::EventLoop::WaitMode::PollForEvents);
}

uint64_t bug_open(char const* url, char const* document_start_script)
{
    if (!application || !url || !document_start_script)
        return 0;
    auto parsed_url = WebView::sanitize_url(StringView(url, strlen(url)), WebView::Application::settings().search_engine_settings().engine);
    if (!parsed_url.has_value())
        return 0;
    auto script = String::from_utf8(StringView(document_start_script, strlen(document_start_script)));
    if (script.is_error()) return 0;
    auto& view = new_view();
    if (!script.value().is_empty()) view.set_document_start_script(script.release_value());
    view.load(*parsed_url);
    return view.view_id();
}

int bug_navigate(uint64_t id, char const* url)
{
    auto* view = lookup(id);
    if (!view || !url)
        return 0;
    auto parsed_url = WebView::sanitize_url(StringView(url, strlen(url)), WebView::Application::settings().search_engine_settings().engine);
    if (!parsed_url.has_value())
        return 0;
    view->cancel_requests();
    view->load(*parsed_url);
    return 1;
}

int bug_reply_file_picker(uint64_t view, uint64_t request, char const* const* paths, size_t count)
{
    auto* target = lookup(view);
    return target ? target->reply_file_picker(request, paths, count) : 0;
}

int bug_reply_request(uint64_t view, uint64_t request, char const* kind, int answer, char const* text, size_t length)
{
    auto* target = lookup(view);
    if (!target || !kind || !text) return 0;
    return target->reply(request, StringView(kind, strlen(kind)), answer, StringView(text, length));
}

int bug_action(uint64_t id, uint32_t action, char const* text, double number)
{
    auto* view = lookup(id);
    if (!view)
        return 0;
    switch (action) {
    case BUG_SET_REQUEST_HANDLERS:
        return text && view->set_handlers(StringView(text, strlen(text)));
    case BUG_DOCUMENT_START_SCRIPT: {
        if (!text) return 0;
        auto script = String::from_utf8(StringView(text, strlen(text)));
        if (script.is_error()) return 0;
        view->set_document_start_script(script.release_value());
        break;
    }
    case BUG_BACK:
        view->cancel_requests();
        view->traverse_the_history_by_delta(-1);
        break;
    case BUG_FORWARD:
        view->cancel_requests();
        view->traverse_the_history_by_delta(1);
        break;
    case BUG_RELOAD:
        view->cancel_requests();
        view->reload();
        break;
    case BUG_STOP: view->stop_loading(); break;
    case BUG_CLOSE:
        view->cancel_requests();
        view->request_close();
        break;
    case BUG_EVAL: {
        if (!text) return 0;
        auto script = String::from_utf8(StringView(text, strlen(text)));
        if (script.is_error()) return 0;
        view->run_javascript(script.release_value());
        break;
    }
    case BUG_ZOOM:
        if (!std::isfinite(number) || number < 0.25 || number > 5.0) return 0;
        view->set_zoom(number); break;
    case BUG_FOCUS: view->set_has_system_focus(true); break;
    case BUG_BLUR: view->set_has_system_focus(false); break;
    case BUG_COPY:
    case BUG_CUT: {
        auto promise = action == BUG_COPY ? view->selected_text() : view->cut_selected_text();
        promise->when_resolved([](ByteString const& text) {
            if (!text.is_empty())
                WebView::Application::the().insert_clipboard_entry({ "text/plain"_string, text });
        });
        break;
    }
    case BUG_PASTE: {
        if (!text) return 0;
        auto value = String::from_utf8(StringView(text, strlen(text)));
        if (value.is_error()) return 0;
        WebView::Application::the().set_clipboard_text(value.release_value());
        view->paste_from_clipboard(Web::Clipboard::PasteMode::PlainTextOnly);
        break;
    }
    default: return 0;
    }
    return 1;
}

int bug_evaluate(uint64_t id, uint64_t request, char const* body)
{
    auto* view = lookup(id);
    if (!view || !body || !request || evaluation_views.contains(request)) return 0;
    // WebDriver would otherwise try to dismiss the controller's dialog before evaluation.
    if (view->has_modal_request()) return -1;
    auto script = String::from_utf8(StringView(body, strlen(body)));
    if (script.is_error()) return 0;
    JsonObject payload;
    payload.set("script"sv, script.release_value());
    payload.set("args"sv, JsonArray {});
    evaluation_views.set(request, id);
    view->run_webdriver_content_command(request, Web::WebDriver::SessionBrowsingContext::CurrentTopLevel,
        "execute_script"_string, move(payload), {});
    return 1;
}

int bug_resize(uint64_t id, uint32_t width, uint32_t height, double scale)
{
    auto* view = lookup(id);
    if (!view || !width || !height || width > 16384 || height > 16384 || !std::isfinite(scale) || scale < 0.25 || scale > 8)
        return 0;
    view->resize(width, height, scale);
    return 1;
}

size_t bug_frame(uint64_t id, uint32_t* dest, size_t capacity, uint32_t* width, uint32_t* height)
{
    if (!width || !height) return 0;
    *width = *height = 0;
    auto* view = lookup(id);
    return view ? view->copy_frame(dest, capacity, *width, *height) : 0;
}

int bug_mouse(uint64_t id, uint32_t kind, int32_t x, int32_t y, uint32_t button, uint32_t buttons,
    uint32_t modifiers, double dx, double dy, uint32_t clicks)
{
    auto* view = lookup(id);
    if (!view || kind > 4 || !std::isfinite(dx) || !std::isfinite(dy)) return 0;
    using Type = Web::MouseEvent::Type;
    Type types[] = { Type::MouseMove, Type::MouseDown, Type::MouseUp, Type::MouseWheel, Type::MouseLeave };
    view->enqueue_input_event(Web::MouseEvent {
        .type = types[kind],
        .position = { x, y },
        .screen_position = { x, y },
        .button = static_cast<Web::UIEvents::MouseButton>(button & 31),
        .buttons = static_cast<Web::UIEvents::MouseButton>(buttons & 31),
        .modifiers = static_cast<Web::UIEvents::KeyModifier>(modifiers & 15),
        .wheel_delta_x = dx,
        .wheel_delta_y = dy,
        .wheel_tick_120ths_x = dx * 3,
        .wheel_tick_120ths_y = dy * 3,
        .wheel_delta_precision = Web::WheelDeltaPrecision::Precise,
        .click_count = static_cast<int>(clicks),
    });
    return 1;
}

int bug_key(uint64_t id, int pressed, char const* name, uint32_t codepoint, uint32_t modifiers, int repeat)
{
    auto* view = lookup(id);
    if (!view || !name || codepoint > 0x10ffff || (codepoint >= 0xd800 && codepoint <= 0xdfff)) return 0;
    auto key = Web::UIEvents::Key_Invalid;
#define __ENUMERATE_KEY_CODE(symbol, label, value) if (strcmp(name, label) == 0) key = Web::UIEvents::Key_##symbol;
    ENUMERATE_KEY_CODES
#undef __ENUMERATE_KEY_CODE
    view->enqueue_input_event(Web::KeyEvent {
        .type = pressed ? Web::KeyEvent::Type::KeyDown : Web::KeyEvent::Type::KeyUp,
        .key = key,
        .modifiers = static_cast<Web::UIEvents::KeyModifier>(modifiers & 15),
        .code_point = codepoint,
        .repeat = repeat != 0,
        .should_insert_text = pressed && codepoint >= 32 && !(modifiers & (2 | 8)),
    });
    return 1;
}
}
