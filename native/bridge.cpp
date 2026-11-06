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
#include <LibWebCommon/HTML/VisibilityState.h>
#include <LibWebCommon/Page/InputEvent.h>
#include <LibWebCommon/WebView/Utilities.h>
#include <LibWebView/Application.h>
#include <LibWebView/ViewImplementation.h>

#include <cstring>
#include <cmath>

namespace {
BugEvent callback;
void* callback_user;
Core::AnonymousBuffer theme;
HashMap<u64, u64> evaluation_views;

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
        on_ready_to_paint = [this] { emit(BUG_FRAME, view_id()); };
        on_title_change = [this](auto const& title) { emit(BUG_TITLE, view_id(), title.to_utf8()); };
        on_url_change = [this](auto const& url) { emit(BUG_URL, view_id(), url.serialize()); };
        on_load_finish = [this](auto const& url) { emit(BUG_LOADED, view_id(), url.serialize()); };
        on_close = [this] {
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
        on_web_content_crashed = [this](auto) { emit(BUG_ERROR, view_id(), "renderer crashed; reload to retry"sv); };

        // Until a controller supplies dialog replies, settle requests rather than
        // leaving the renderer blocked. Report each unsupported interaction.
        on_request_alert = [this](auto const& message) {
            emit(BUG_HOOK, view_id(), ByteString::formatted("alert {}", message.to_utf8()));
            alert_closed();
        };
        on_request_confirm = [this](auto const& message) {
            emit(BUG_HOOK, view_id(), ByteString::formatted("confirm-dismissed {}", message.to_utf8()));
            confirm_closed(false);
        };
        on_request_prompt = [this](auto const& message, auto const&) {
            emit(BUG_HOOK, view_id(), ByteString::formatted("prompt-dismissed {}", message.to_utf8()));
            prompt_closed({});
        };
        on_request_before_unload = [this](auto const&, auto done) {
            emit(BUG_HOOK, view_id(), "before-unload-accepted"sv);
            done(true);
        };
        on_request_permission = [this](auto const&, auto done) {
            emit(BUG_HOOK, view_id(), "permission-dismissed"sv);
            done(Web::Permissions::PromptOutcome::Dismissed);
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

    size_t copy_frame(uint32_t* dest, size_t capacity, uint32_t& width, uint32_t& height)
    {
        width = height = 0;
        if (!m_has_usable_bitmap || !m_front_bitmap.shared_image_buffer)
            return 0;
        auto bitmap = m_front_bitmap.shared_image_buffer->bitmap_if_present();
        if (!bitmap)
            return 0;
        width = min(bitmap->width(), m_front_bitmap.last_painted_size.width().value());
        height = min(bitmap->height(), m_front_bitmap.last_painted_size.height().value());
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
            error.set("name"sv, response.error().error);
            error.set("message"sv, response.error().message);
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
uint32_t bug_abi_version() { return 2; }

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

uint64_t bug_open(char const* url)
{
    if (!application || !url)
        return 0;
    auto& view = new_view();
    view.load_from_user_input(StringView(url, strlen(url)));
    return view.view_id();
}

int bug_navigate(uint64_t id, char const* url)
{
    auto* view = lookup(id);
    if (!view || !url)
        return 0;
    view->load_from_user_input(StringView(url, strlen(url)));
    return 1;
}

int bug_action(uint64_t id, uint32_t action, char const* text, double number)
{
    auto* view = lookup(id);
    if (!view)
        return 0;
    switch (action) {
    case BUG_BACK: view->traverse_the_history_by_delta(-1); break;
    case BUG_FORWARD: view->traverse_the_history_by_delta(1); break;
    case BUG_RELOAD: view->reload(); break;
    case BUG_STOP: view->stop_loading(); break;
    case BUG_CLOSE: view->request_close(); break;
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
        view->paste_from_clipboard();
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
