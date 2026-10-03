pub fn invoke<T: plain_helper::Run<P>, P>(runner: &T, provider: &P) {
    plain_helper::invoke(runner, provider);
}
