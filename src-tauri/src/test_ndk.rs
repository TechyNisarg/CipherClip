fn main() {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.unwrap();
    let activity = unsafe { jni::objects::JObject::from_raw(ctx.context().cast()) };
}
