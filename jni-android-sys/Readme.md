# jni-android-sys

**Work in progress, only barely kinda partially usable, APIs not yet stabilized**

Uses [jni-bindgen](https://github.com/MaulingMonkey/jni-bindgen) to export Android's Java APIs to Rust.
Only tested against Android API level 28 so far.

### Cargo.toml

```toml
[dependencies]
jni-android-sys = { version = "0.0.7", features = ["api-level-28", "android-view-KeyEvent"] }
```

### MainActivity.java

```java
package com.example;

import androidx.appcompat.app.AppCompatActivity;
import android.view.KeyEvent;

public class MainActivity extends AppCompatActivity {
    static { System.loadLibrary("example"); }
    @Override public native boolean dispatchKeyEvent(KeyEvent keyEvent);
}
```

### main_activity.rs

```rust
use jni_sys::{jboolean, jobject, JNI_TRUE};
use jni_glue::{Argument, Env};
use jni_android_sys::android::view::KeyEvent;

#[no_mangle] pub extern "system" fn Java_com_example_MainActivity_dispatchKeyEvent(
    env:        &Env,
    _this:      jobject,
    key_event:  Argument<KeyEvent>,
) -> jboolean {
    let key_event = unsafe { key_event.with_unchecked(env) }; // Unsafe boilerplate not yet autogenerated.

    if let Some(key_event) = key_event {
        // Err = Java exception was thrown.
        let is_enter = key_event.get_key_code() == Ok(KeyEvent::KEYCODE_ENTER);
        let is_down  = key_event.get_action() == Ok(KeyEvent::ACTION_DOWN);
        if is_enter && is_down {
            println!("ENTER pressed"); // Not that you can see this...
        }
    }

    JNI_TRUE // JNI boilerplate not yet autogenerated
}
```

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

<!-- https://doc.rust-lang.org/1.4.0/complement-project-faq.html#why-dual-mit/asl2-license? -->
<!-- https://rust-lang-nursery.github.io/api-guidelines/necessities.html#crate-and-its-dependencies-have-a-permissive-license-c-permissive -->
<!-- https://choosealicense.com/licenses/apache-2.0/ -->
<!-- https://choosealicense.com/licenses/mit/ -->

## Build Features

| feature                               | description   |
| ------------------------------------- | ------------- |
| `"api-level-28"`                      | Define android APIs as they were defined in API level 28 or greater
| `"force-define"`                      | Define android APIs on non-android targets
| `"android-view-KeyEvent"`             | Define the android.view.[KeyEvent](https://developer.android.com/reference/android/view/KeyEvent.html) class
| `"android-view-KeyEvent_Callback"`    | Define the android.view.[KeyEvent.Callback](https://developer.android.com/reference/android/view/KeyEvent.Callback.html) interface
| ...thousands of other features...     | Define other android.\*, androidx.\*, dalvik.\*, java.\*, javax.\*, and org.\* APIs.
| `"all"`                               | Define all the available android/java APIs